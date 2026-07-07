//! Resilient concurrent download engine.
//!
//! Given the file list from [`ApiClient::list_download_files`], fetch each
//! presigned URL and write it to a rendered path template. Downloads run with
//! bounded concurrency, retry transient failures, stream to a temp file, and
//! rename atomically over the destination. Individual failures do not abort the
//! batch; they are collected into a [`DownloadReport`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use backon::Retryable;
use futures::StreamExt;
use futures::stream;
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

use crate::api::{ApiClient, ApiClientConfig, parse_retry_after};
use crate::error::Error;
use crate::model::{DownloadRequest, DownloadableFile};
use crate::retry::RetryPolicy;
use crate::template;

/// Default number of files fetched concurrently.
pub const DEFAULT_CONCURRENCY: usize = 8;

/// Everything needed to run a download.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub api_key: String,
    pub base_url: String,
    /// Path template with `{lang}` / `{ns}` / … placeholders.
    pub download_path: String,
    pub request: DownloadRequest,
    pub concurrency: usize,
    pub request_timeout: Option<Duration>,
    pub retry: RetryPolicy,
}

/// A successfully downloaded file.
#[derive(Debug, Clone)]
pub struct DownloadedFile {
    pub path: PathBuf,
    pub language: Option<String>,
    pub namespace: Option<String>,
    pub bytes: u64,
}

/// A file that could not be downloaded.
#[derive(Debug, Clone)]
pub struct FailedFile {
    pub url: String,
    pub language: Option<String>,
    pub namespace: Option<String>,
    pub error: String,
}

/// Outcome of a batch: what landed on disk and what failed.
#[derive(Debug, Clone, Default)]
pub struct DownloadReport {
    pub downloaded: Vec<DownloadedFile>,
    pub failed: Vec<FailedFile>,
}

impl DownloadReport {
    /// True when every file downloaded successfully.
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Progress events emitted to an optional callback during a download.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Started { total: usize },
    FileDone { path: PathBuf, bytes: u64 },
    FileFailed { url: String, error: String },
    Finished { downloaded: usize, failed: usize },
}

/// Sink for [`DownloadEvent`]s. Kept sync-callback-friendly for the Python layer.
pub type ProgressCallback = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

fn emit(callback: &Option<ProgressCallback>, event: DownloadEvent) {
    if let Some(callback) = callback {
        callback(event);
    }
}

/// Orchestration entry point: list files, then fetch and write them all.
///
/// Returns `Err` only for pre-flight failures (the list call: auth, network,
/// bad request). Once the batch starts, per-file failures are collected into the
/// returned [`DownloadReport`]; callers decide whether a non-empty
/// `report.failed` should be treated as an error.
pub async fn download(
    config: DownloadConfig,
    on_event: Option<ProgressCallback>,
) -> Result<DownloadReport, Error> {
    let client = ApiClient::new(ApiClientConfig {
        api_key: config.api_key.clone(),
        base_url: config.base_url.clone(),
        request_timeout: config.request_timeout,
        retry: config.retry.clone(),
    })?;

    let files = client.list_download_files(&config.request).await?;
    Ok(download_all(
        &client,
        files,
        &config.download_path,
        config.concurrency,
        on_event,
    )
    .await)
}

async fn download_all(
    client: &ApiClient,
    files: Vec<DownloadableFile>,
    template: &str,
    concurrency: usize,
    on_event: Option<ProgressCallback>,
) -> DownloadReport {
    let total = files.len();
    emit(&on_event, DownloadEvent::Started { total });
    log::info!("downloading {total} file(s) with concurrency {concurrency}");

    let concurrency = concurrency.max(1);
    let outcomes = stream::iter(files.into_iter().map(|file| {
        let client = client.clone();
        let template = template.to_string();
        let on_event = on_event.clone();
        async move {
            match download_one(&client, &file, &template).await {
                Ok((path, bytes)) => {
                    emit(
                        &on_event,
                        DownloadEvent::FileDone {
                            path: path.clone(),
                            bytes,
                        },
                    );
                    Ok(DownloadedFile {
                        path,
                        language: file.language.clone(),
                        namespace: file.namespace.clone(),
                        bytes,
                    })
                }
                Err(err) => {
                    let error = err.to_string();
                    log::warn!("failed to download {}: {error}", file.url);
                    emit(
                        &on_event,
                        DownloadEvent::FileFailed {
                            url: file.url.clone(),
                            error: error.clone(),
                        },
                    );
                    Err(FailedFile {
                        url: file.url.clone(),
                        language: file.language.clone(),
                        namespace: file.namespace.clone(),
                        error,
                    })
                }
            }
        }
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await;

    let mut report = DownloadReport::default();
    for outcome in outcomes {
        match outcome {
            Ok(file) => report.downloaded.push(file),
            Err(file) => report.failed.push(file),
        }
    }
    emit(
        &on_event,
        DownloadEvent::Finished {
            downloaded: report.downloaded.len(),
            failed: report.failed.len(),
        },
    );
    log::info!(
        "download finished: {} ok, {} failed",
        report.downloaded.len(),
        report.failed.len()
    );
    report
}

async fn download_one(
    client: &ApiClient,
    file: &DownloadableFile,
    template: &str,
) -> Result<(PathBuf, u64), Error> {
    let out_path = template::resolve_output_path(template, file)?;
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::io(parent, e))?;
    }

    let bytes = (|| fetch_and_write(client, &file.url, &out_path))
        .retry(client.retry_policy().builder())
        .when(Error::is_retryable)
        // Honor a server `Retry-After`, falling back to the computed backoff.
        .adjust(|err, dur| err.retry_after().or(dur))
        .await?;
    Ok((out_path, bytes))
}

/// One fetch-and-write attempt. Streams the body to a temp file in the
/// destination directory and renames it over the destination on success.
/// Retryability (transient transport / 429 / 5xx) is decided by the caller via
/// [`Error::is_retryable`].
async fn fetch_and_write(client: &ApiClient, url: &str, out_path: &Path) -> Result<u64, Error> {
    // Presigned URLs are fetched without the auth header. A transport failure
    // here surfaces as `Error::Network` and is retried if transient.
    let response = client.http().get(url).send().await?;

    let status = response.status();
    if !status.is_success() {
        let code = status.as_u16();
        let msg = format!("file fetch failed: HTTP {code}");
        // 5xx / 429 are retryable (via `is_retryable`), and carry any
        // `Retry-After`. 403 on a presigned URL usually means the URL expired;
        // refreshing the file list is a documented future improvement, so for
        // now it (and other 4xx) fail fast for this file.
        let retry_after = if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            parse_retry_after(response.headers())
        } else {
            None
        };
        return Err(Error::Api {
            status: code,
            msg,
            retry_after,
        });
    }

    let tmp_path = temp_path_for(out_path);
    let mut tmp_file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|err| Error::io(&tmp_path, err))?;

    let mut stream = response.bytes_stream();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                if let Err(err) = tmp_file.write_all(&bytes).await {
                    cleanup(&tmp_path).await;
                    return Err(Error::io(&tmp_path, err));
                }
                total += bytes.len() as u64;
            }
            Err(err) => {
                cleanup(&tmp_path).await;
                // A mid-stream transport error is a retryable body error.
                return Err(Error::Network(err));
            }
        }
    }

    if let Err(err) = tmp_file.flush().await {
        cleanup(&tmp_path).await;
        return Err(Error::io(&tmp_path, err));
    }
    drop(tmp_file);

    if total == 0 {
        log::warn!("empty response body written for {url}");
    }

    if let Err(err) = tokio::fs::rename(&tmp_path, out_path).await {
        cleanup(&tmp_path).await;
        return Err(Error::io(out_path, err));
    }
    Ok(total)
}

async fn cleanup(tmp_path: &Path) {
    let _ = tokio::fs::remove_file(tmp_path).await;
}

/// A unique temp path in the same directory as `out_path` (so the final rename
/// is atomic on the same filesystem).
fn temp_path_for(out_path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = out_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp_name = format!(".{name}.tmp-{pid}-{seq}");
    match out_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DownloadFormat;
    use std::collections::HashSet;
    use tempfile::TempDir;
    use tokio::time::Instant;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast_retry() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 4,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            factor: 2,
        }
    }

    fn config_at(server: &MockServer, dir: &TempDir, concurrency: usize) -> DownloadConfig {
        DownloadConfig {
            api_key: "key".into(),
            base_url: server.uri(),
            download_path: format!("{}/{{lang}}/{{ns}}.json", dir.path().display()),
            request: DownloadRequest {
                format: DownloadFormat::SingleLanguageJson,
                ..Default::default()
            },
            concurrency,
            request_timeout: Some(Duration::from_secs(5)),
            retry: fast_retry(),
        }
    }

    /// Mount the `/cli/v2/download` list returning `files` (each URL points back
    /// at the mock server under `/files/...`).
    async fn mount_list(server: &MockServer, files: &[(&str, &str)]) {
        let entries: Vec<_> = files
            .iter()
            .map(|(lang, ns)| {
                serde_json::json!({
                    "url": format!("{}/files/{lang}/{ns}.json", server.uri()),
                    "language": lang,
                    "namespace": ns,
                })
            })
            .collect();
        Mock::given(method("GET"))
            .and(path("/cli/v2/download"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "files": entries
            })))
            .mount(server)
            .await;
    }

    fn tmp_files(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in walk(dir) {
            if entry
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".tmp-"))
            {
                found.push(entry);
            }
        }
        found
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(read) = std::fs::read_dir(dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    out.extend(walk(&path));
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    #[tokio::test]
    async fn downloads_tree_with_correct_contents() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        mount_list(
            &server,
            &[("en", "common"), ("ja", "common"), ("en", "auth")],
        )
        .await;
        for (lang, ns) in [("en", "common"), ("ja", "common"), ("en", "auth")] {
            Mock::given(method("GET"))
                .and(path(format!("/files/{lang}/{ns}.json")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(format!("{{\"k\":\"{lang}-{ns}\"}}")),
                )
                .mount(&server)
                .await;
        }

        let report = download(config_at(&server, &dir, 4), None).await.unwrap();
        assert!(report.is_success());
        assert_eq!(report.downloaded.len(), 3);

        let en_common = std::fs::read_to_string(dir.path().join("en/common.json")).unwrap();
        assert_eq!(en_common, "{\"k\":\"en-common\"}");
        assert!(dir.path().join("ja/common.json").exists());
        assert!(dir.path().join("en/auth.json").exists());
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn one_hard_failure_is_reported_others_succeed() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        mount_list(&server, &[("en", "ok"), ("ja", "bad")]).await;
        Mock::given(method("GET"))
            .and(path("/files/en/ok.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/files/ja/bad.json"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let report = download(config_at(&server, &dir, 4), None).await.unwrap();
        assert!(!report.is_success());
        assert_eq!(report.downloaded.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].namespace.as_deref(), Some("bad"));
        assert!(dir.path().join("en/ok.json").exists());
        assert!(!dir.path().join("ja/bad.json").exists());
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn transient_5xx_on_file_retries_then_succeeds() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        mount_list(&server, &[("en", "flaky")]).await;
        Mock::given(method("GET"))
            .and(path("/files/en/flaky.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":true}"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/files/en/flaky.json"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let report = download(config_at(&server, &dir, 2), None).await.unwrap();
        assert!(report.is_success(), "failures: {:?}", report.failed);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("en/flaky.json")).unwrap(),
            "{\"ok\":true}"
        );
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn overwrites_existing_file() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("en")).unwrap();
        std::fs::write(dir.path().join("en/common.json"), "OLD").unwrap();

        mount_list(&server, &[("en", "common")]).await;
        Mock::given(method("GET"))
            .and(path("/files/en/common.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("NEW"))
            .mount(&server)
            .await;

        download(config_at(&server, &dir, 1), None).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("en/common.json")).unwrap(),
            "NEW"
        );
    }

    #[tokio::test]
    async fn traversal_in_metadata_is_reported_not_written() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        // namespace escapes upward.
        mount_list(&server, &[("en", "../../evil")]).await;
        Mock::given(method("GET"))
            .and(path("/files/en/../../evil.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x"))
            .mount(&server)
            .await;

        let report = download(config_at(&server, &dir, 1), None).await.unwrap();
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].error.contains("path"));
    }

    #[tokio::test]
    async fn emits_progress_events() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        mount_list(&server, &[("en", "a"), ("en", "b")]).await;
        for ns in ["a", "b"] {
            Mock::given(method("GET"))
                .and(path(format!("/files/en/{ns}.json")))
                .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
                .mount(&server)
                .await;
        }

        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        let cb: ProgressCallback = Arc::new(move |ev| sink.lock().unwrap().push(ev));

        download(config_at(&server, &dir, 2), Some(cb))
            .await
            .unwrap();
        let events = events.lock().unwrap();
        let started = events
            .iter()
            .any(|e| matches!(e, DownloadEvent::Started { total: 2 }));
        let done = events
            .iter()
            .filter(|e| matches!(e, DownloadEvent::FileDone { .. }))
            .count();
        let finished = events.iter().any(|e| {
            matches!(
                e,
                DownloadEvent::Finished {
                    downloaded: 2,
                    failed: 0
                }
            )
        });
        assert!(started && finished);
        assert_eq!(done, 2);
    }

    #[tokio::test]
    async fn respects_concurrency_limit() {
        let server = MockServer::start().await;
        let dir = TempDir::new().unwrap();
        let files: Vec<(String, String)> = (0..8)
            .map(|i| ("en".to_string(), format!("ns{i}")))
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(l, n)| (l.as_str(), n.as_str()))
            .collect();
        mount_list(&server, &refs).await;
        for (_, ns) in &refs {
            Mock::given(method("GET"))
                .and(path(format!("/files/en/{ns}.json")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string("{}")
                        .set_delay(Duration::from_millis(100)),
                )
                .mount(&server)
                .await;
        }

        // 8 files @100ms: sequential ≈ 800ms, concurrency 4 ≈ 200ms. A generous
        // ceiling well under the sequential time proves work overlapped.
        let start = Instant::now();
        let report = download(config_at(&server, &dir, 4), None).await.unwrap();
        let elapsed = start.elapsed();
        assert_eq!(report.downloaded.len(), 8);
        assert!(
            elapsed < Duration::from_millis(600),
            "elapsed {elapsed:?} suggests downloads did not run concurrently"
        );
        // Every namespace is distinct → 8 files on disk.
        let names: HashSet<_> = report.downloaded.iter().map(|f| f.path.clone()).collect();
        assert_eq!(names.len(), 8);
    }
}
