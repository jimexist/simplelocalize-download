//! HTTP client for the SimpleLocalize `/cli/v2/download` endpoint.

use std::time::Duration;

use backon::Retryable;
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Client, Response, StatusCode};

use crate::error::Error;
use crate::model::{DownloadListResponse, DownloadRequest, DownloadableFile};
use crate::retry::RetryPolicy;

/// Default SimpleLocalize API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.simplelocalize.io";
/// Version reported to the API (and used as part of the user agent).
pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

const TOKEN_HEADER: &str = "X-SimpleLocalize-Token";
const VERSION_HEADER: &str = "X-SimpleLocalize-Cli-Version";
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration for [`ApiClient`].
#[derive(Debug, Clone)]
pub struct ApiClientConfig {
    pub api_key: String,
    pub base_url: String,
    /// Overall per-request timeout. Defaults to 5 minutes when `None`.
    pub request_timeout: Option<Duration>,
    pub retry: RetryPolicy,
}

impl ApiClientConfig {
    /// Config with the default base URL and retry policy.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            request_timeout: None,
            retry: RetryPolicy::default(),
        }
    }
}

/// Client for the SimpleLocalize CLI API. Holds a shared [`reqwest::Client`]
/// (connection pooling) and a retry policy.
#[derive(Clone)]
pub struct ApiClient {
    http: Client,
    base_url: String,
    api_key: String,
    retry: RetryPolicy,
}

impl ApiClient {
    /// Build a client. `reqwest` honors `http_proxy`/`https_proxy`/`no_proxy`
    /// from the environment automatically.
    pub fn new(config: ApiClientConfig) -> Result<Self, Error> {
        let http = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(config.request_timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT))
            .user_agent(format!("simplelocalize-download/{CLI_VERSION}"))
            .build()?;
        Ok(Self {
            http,
            base_url: config.base_url,
            api_key: config.api_key,
            retry: config.retry,
        })
    }

    /// The shared HTTP client (reused by the download engine for file fetches).
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// The configured retry policy.
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Call `GET /cli/v2/download` and return the list of files to fetch.
    pub async fn list_download_files(
        &self,
        req: &DownloadRequest,
    ) -> Result<Vec<DownloadableFile>, Error> {
        let url = format!("{}/cli/v2/download", self.base_url.trim_end_matches('/'));
        let query = build_query(req);
        log::debug!("listing download files: {url}");
        (|| self.list_once(&url, &query))
            .retry(self.retry.builder())
            .when(Error::is_retryable)
            // Honor a server `Retry-After`, falling back to the computed backoff.
            .adjust(|err, dur| err.retry_after().or(dur))
            .notify(|err, dur| {
                log::debug!("retryable error, backing off {dur:?}: {err}");
            })
            .await
    }

    async fn list_once(
        &self,
        url: &str,
        query: &[(&'static str, String)],
    ) -> Result<Vec<DownloadableFile>, Error> {
        let resp = self
            .http
            .get(url)
            .query(query)
            .header(TOKEN_HEADER, &self.api_key)
            .header(VERSION_HEADER, CLI_VERSION)
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            return resp
                .json::<DownloadListResponse>()
                .await
                .map(|body| body.files)
                .map_err(|err| Error::InvalidResponse(format!("failed to decode: {err}")));
        }

        let retry_after = parse_retry_after(resp.headers());
        let msg = extract_error_message(resp).await;
        Err(classify_status(status, msg, retry_after))
    }
}

/// Map a non-success HTTP status to an [`Error`]: 401/403 auth, everything else
/// an API error. Whether the caller retries is decided by [`Error::is_retryable`]
/// (429/5xx retryable, other 4xx permanent).
fn classify_status(status: StatusCode, msg: String, retry_after: Option<Duration>) -> Error {
    let code = status.as_u16();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        Error::Auth { status: code, msg }
    } else {
        Error::Api {
            status: code,
            msg,
            retry_after,
        }
    }
}

pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Extract `$.msg` from a JSON error body, mirroring the Java CLI. Falls back to
/// a generic message that never leaks response internals.
async fn extract_error_message(resp: Response) -> String {
    let status = resp.status().as_u16();
    let fallback = format!("Unknown error, HTTP Status: {status}");
    match resp.text().await {
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("msg")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or(fallback),
        Err(_) => fallback,
    }
}

fn build_query(req: &DownloadRequest) -> Vec<(&'static str, String)> {
    let mut query: Vec<(&'static str, String)> =
        vec![("downloadFormat", req.format.as_str().to_string())];
    if !req.language_keys.is_empty() {
        query.push(("languageKey", req.language_keys.join(",")));
    }
    if !req.options.is_empty() {
        query.push(("downloadOptions", req.options.join(",")));
    }
    if let Some(sort) = &req.sort {
        query.push(("sort", sort.clone()));
    }
    if let Some(namespace) = &req.namespace {
        query.push(("namespace", namespace.clone()));
    }
    if !req.tags.is_empty() {
        query.push(("tags", req.tags.join(",")));
    }
    if let Some(customer_id) = &req.customer_id {
        query.push(("customerId", customer_id.clone()));
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DownloadFormat;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> ApiClient {
        ApiClient::new(ApiClientConfig {
            api_key: "secret-key".into(),
            base_url: server.uri(),
            request_timeout: Some(Duration::from_secs(5)),
            retry: RetryPolicy {
                max_attempts: 4,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
                factor: 2,
            },
        })
        .unwrap()
    }

    #[test]
    fn build_query_omits_empty_fields() {
        let req = DownloadRequest {
            format: DownloadFormat::SingleLanguageJson,
            ..Default::default()
        };
        assert_eq!(
            build_query(&req),
            vec![("downloadFormat", "single-language-json".to_string())]
        );
    }

    #[test]
    fn build_query_joins_collections() {
        let req = DownloadRequest {
            format: DownloadFormat::SingleLanguageJson,
            language_keys: vec!["en".into(), "ja".into()],
            options: vec!["WRITE_NESTED".into()],
            sort: Some("LEXICOGRAPHICAL".into()),
            tags: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        let query = build_query(&req);
        assert!(query.contains(&("languageKey", "en,ja".to_string())));
        assert!(query.contains(&("downloadOptions", "WRITE_NESTED".to_string())));
        assert!(query.contains(&("sort", "LEXICOGRAPHICAL".to_string())));
        assert!(query.contains(&("tags", "a,b".to_string())));
    }

    #[tokio::test]
    async fn happy_path_sends_headers_and_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/cli/v2/download"))
            .and(query_param("downloadFormat", "single-language-json"))
            .and(query_param("downloadOptions", "WRITE_NESTED"))
            .and(query_param("sort", "LEXICOGRAPHICAL"))
            .and(header(TOKEN_HEADER, "secret-key"))
            .and(header(VERSION_HEADER, CLI_VERSION))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "files": [
                    {"url": "https://cdn/en.json", "language": "en", "namespace": "common"},
                    {"url": "https://cdn/ja.json", "language": "ja", "namespace": "common"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server);
        let req = DownloadRequest {
            options: vec!["WRITE_NESTED".into()],
            sort: Some("LEXICOGRAPHICAL".into()),
            ..Default::default()
        };
        let files = client.list_download_files(&req).await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].language.as_deref(), Some("en"));
        assert_eq!(files[1].url, "https://cdn/ja.json");
    }

    #[tokio::test]
    async fn empty_files_and_unknown_fields() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "files": [],
                "unexpected": {"nested": true}
            })))
            .mount(&server)
            .await;
        let files = client_for(&server)
            .list_download_files(&DownloadRequest::default())
            .await
            .unwrap();
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn unauthorized_is_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "msg": "Invalid token"
            })))
            .mount(&server)
            .await;
        let err = client_for(&server)
            .list_download_files(&DownloadRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Auth { status: 401, .. }));
    }

    #[tokio::test]
    async fn bad_request_carries_server_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "msg": "Unknown download format"
            })))
            .mount(&server)
            .await;
        let err = client_for(&server)
            .list_download_files(&DownloadRequest::default())
            .await
            .unwrap_err();
        match err {
            Error::Api { status: 400, msg, .. } => assert_eq!(msg, "Unknown download format"),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn retries_5xx_then_succeeds() {
        let server = MockServer::start().await;
        // First mount wins while it has calls remaining (wiremock checks newest first).
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "files": [{"url": "https://cdn/en.json", "language": "en"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        let files = client_for(&server)
            .list_download_files(&DownloadRequest::default())
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
    }

    #[tokio::test]
    async fn persistent_5xx_fails_after_max_attempts() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(4) // matches RetryPolicy.max_attempts in client_for
            .mount(&server)
            .await;
        let err = client_for(&server)
            .list_download_files(&DownloadRequest::default())
            .await
            .unwrap_err();
        assert_eq!(err.status(), Some(500));
    }
}
