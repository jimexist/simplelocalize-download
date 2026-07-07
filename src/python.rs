//! Python bindings for the download engine.
//!
//! Exposes a single synchronous `download(...)` function plus result classes and
//! an exception hierarchy. The download runs on a shared multi-thread tokio
//! runtime with the GIL released, re-acquiring it only to invoke the optional
//! progress callback.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::api::DEFAULT_BASE_URL;
use crate::download::{
    DEFAULT_CONCURRENCY, DownloadConfig, DownloadEvent, DownloadReport, ProgressCallback, download,
};
use crate::error::Error;
use crate::model::{DownloadFormat, DownloadRequest};

const SUPPORTED_FORMAT: &str = "single-language-json";

create_exception!(
    _core,
    SimpleLocalizeError,
    PyException,
    "Base class for all simplelocalize-download errors."
);
create_exception!(
    _core,
    AuthError,
    SimpleLocalizeError,
    "Invalid API credentials."
);
create_exception!(
    _core,
    ApiError,
    SimpleLocalizeError,
    "SimpleLocalize API returned an error."
);
create_exception!(
    _core,
    NetworkError,
    SimpleLocalizeError,
    "A network/transport failure."
);
create_exception!(
    _core,
    DownloadFailedError,
    SimpleLocalizeError,
    "One or more files failed to download; see the `report` attribute."
);

/// A successfully downloaded file.
#[pyclass(name = "DownloadedFile", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
pub struct PyDownloadedFile {
    pub path: String,
    pub language: Option<String>,
    pub namespace: Option<String>,
    pub bytes: u64,
}

#[pymethods]
impl PyDownloadedFile {
    fn __repr__(&self) -> String {
        format!(
            "DownloadedFile(path={:?}, language={:?}, namespace={:?}, bytes={})",
            self.path, self.language, self.namespace, self.bytes
        )
    }
}

/// A file that failed to download.
#[pyclass(name = "FailedFile", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
pub struct PyFailedFile {
    pub url: String,
    pub language: Option<String>,
    pub namespace: Option<String>,
    pub error: String,
}

#[pymethods]
impl PyFailedFile {
    fn __repr__(&self) -> String {
        format!(
            "FailedFile(url={:?}, language={:?}, namespace={:?}, error={:?})",
            self.url, self.language, self.namespace, self.error
        )
    }
}

/// The outcome of a download batch.
#[pyclass(name = "DownloadReport", frozen, get_all, skip_from_py_object)]
#[derive(Clone)]
pub struct PyDownloadReport {
    pub downloaded: Vec<PyDownloadedFile>,
    pub failed: Vec<PyFailedFile>,
}

#[pymethods]
impl PyDownloadReport {
    /// True when every file downloaded successfully.
    fn is_success(&self) -> bool {
        self.failed.is_empty()
    }

    fn __repr__(&self) -> String {
        format!(
            "DownloadReport(downloaded={}, failed={})",
            self.downloaded.len(),
            self.failed.len()
        )
    }
}

impl From<DownloadReport> for PyDownloadReport {
    fn from(report: DownloadReport) -> Self {
        PyDownloadReport {
            downloaded: report
                .downloaded
                .into_iter()
                .map(|f| PyDownloadedFile {
                    path: f.path.display().to_string(),
                    language: f.language,
                    namespace: f.namespace,
                    bytes: f.bytes,
                })
                .collect(),
            failed: report
                .failed
                .into_iter()
                .map(|f| PyFailedFile {
                    url: f.url,
                    language: f.language,
                    namespace: f.namespace,
                    error: f.error,
                })
                .collect(),
        }
    }
}

/// Shared multi-thread tokio runtime, initialized on first use.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
    })
}

/// Download SimpleLocalize translations.
///
/// Synchronous: blocks until the download completes. The GIL is released for the
/// duration; the optional `on_event` callback receives per-file progress dicts.
#[pyfunction]
#[pyo3(name = "download")]
#[pyo3(signature = (
    api_key,
    download_path,
    *,
    download_format = None,
    language_keys = None,
    download_options = None,
    download_sort = None,
    namespace = None,
    tags = None,
    customer_id = None,
    base_url = None,
    concurrency = DEFAULT_CONCURRENCY,
    request_timeout_secs = None,
    on_event = None,
))]
#[allow(clippy::too_many_arguments)]
fn download_py(
    py: Python<'_>,
    api_key: String,
    download_path: String,
    download_format: Option<String>,
    language_keys: Option<Vec<String>>,
    download_options: Option<Vec<String>>,
    download_sort: Option<String>,
    namespace: Option<String>,
    tags: Option<Vec<String>>,
    customer_id: Option<String>,
    base_url: Option<String>,
    concurrency: usize,
    request_timeout_secs: Option<f64>,
    on_event: Option<Py<PyAny>>,
) -> PyResult<PyDownloadReport> {
    let format = download_format.unwrap_or_else(|| SUPPORTED_FORMAT.to_string());
    if format != SUPPORTED_FORMAT {
        return Err(PyValueError::new_err(format!(
            "unsupported download_format {format:?}: this tool only supports {SUPPORTED_FORMAT:?}"
        )));
    }

    // `Duration::try_from_secs_f64` rejects negative, NaN, infinite, and
    // overflowing values, so an invalid timeout surfaces as a Python
    // `ValueError` instead of panicking (which would abort the interpreter).
    let request_timeout = request_timeout_secs
        .map(Duration::try_from_secs_f64)
        .transpose()
        .map_err(|_| {
            PyValueError::new_err(
                "request_timeout_secs must be a non-negative, finite number of seconds",
            )
        })?;

    let config = DownloadConfig {
        api_key,
        base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
        download_path,
        request: DownloadRequest {
            format: DownloadFormat::SingleLanguageJson,
            language_keys: language_keys.unwrap_or_default(),
            options: download_options.unwrap_or_default(),
            sort: download_sort,
            namespace,
            tags: tags.unwrap_or_default(),
            customer_id,
        },
        concurrency,
        request_timeout,
        retry: Default::default(),
    };

    // Build the callback and run the download entirely with the GIL released.
    // The closure captures only `Py<PyAny>` + owned data (all `Ungil`); the
    // trait-object callback is constructed inside so it never crosses the
    // detach boundary.
    let result = py.detach(move || {
        let callback: Option<ProgressCallback> = on_event.map(|cb| {
            Arc::new(move |event: DownloadEvent| {
                Python::attach(|py| {
                    if let Err(err) = dispatch_event(py, &cb, event) {
                        // A raising callback must not abort the download.
                        log::warn!("progress callback raised an exception: {err}");
                    }
                });
            }) as ProgressCallback
        });
        runtime().block_on(download(config, callback))
    });

    match result {
        Ok(report) => {
            let py_report = PyDownloadReport::from(report);
            if py_report.failed.is_empty() {
                Ok(py_report)
            } else {
                Err(download_failed_error(py, py_report))
            }
        }
        Err(err) => Err(to_pyerr(py, err)),
    }
}

fn dispatch_event(py: Python<'_>, callback: &Py<PyAny>, event: DownloadEvent) -> PyResult<()> {
    let dict = event_to_dict(py, event)?;
    callback.bind(py).call1((dict,))?;
    Ok(())
}

fn event_to_dict<'py>(py: Python<'py>, event: DownloadEvent) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    match event {
        DownloadEvent::Started { total } => {
            dict.set_item("type", "started")?;
            dict.set_item("total", total)?;
        }
        DownloadEvent::FileDone { path, bytes } => {
            dict.set_item("type", "file_done")?;
            dict.set_item("path", path.display().to_string())?;
            dict.set_item("bytes", bytes)?;
        }
        DownloadEvent::FileFailed { url, error } => {
            dict.set_item("type", "file_failed")?;
            dict.set_item("url", url)?;
            dict.set_item("error", error)?;
        }
        DownloadEvent::Finished { downloaded, failed } => {
            dict.set_item("type", "finished")?;
            dict.set_item("downloaded", downloaded)?;
            dict.set_item("failed", failed)?;
        }
    }
    Ok(dict)
}

/// Map a core [`Error`] to the matching Python exception.
///
/// `AuthError` and `ApiError` always expose a `.status` attribute (an `int` when
/// the HTTP status is known, otherwise `None`) so callers can rely on it being
/// present, matching the type stubs.
fn to_pyerr(py: Python<'_>, err: Error) -> PyErr {
    let message = err.to_string();
    match &err {
        Error::Auth { status, .. } => with_status(py, AuthError::new_err(message), Some(*status)),
        Error::Api { status, .. } => with_status(py, ApiError::new_err(message), Some(*status)),
        // A malformed response is surfaced as an ApiError without a status code.
        Error::InvalidResponse(_) => with_status(py, ApiError::new_err(message), None),
        Error::Network(_) => NetworkError::new_err(message),
        Error::Io { .. } | Error::UnsafePath(_) => SimpleLocalizeError::new_err(message),
    }
}

/// Attach a `.status` attribute (`int` or `None`) to an exception.
fn with_status(py: Python<'_>, pyerr: PyErr, status: Option<u16>) -> PyErr {
    let _ = pyerr.value(py).setattr("status", status);
    pyerr
}

fn download_failed_error(py: Python<'_>, report: PyDownloadReport) -> PyErr {
    let failed = report.failed.len();
    let downloaded = report.downloaded.len();
    let err = DownloadFailedError::new_err(format!(
        "{failed} file(s) failed to download ({downloaded} succeeded)"
    ));
    match Py::new(py, report) {
        Ok(obj) => {
            let _ = err.value(py).setattr("report", obj);
        }
        Err(inner) => log::error!("failed to attach report to DownloadFailedError: {inner}"),
    }
    err
}

/// Register everything into the `_core` module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Forward Rust `log` records to Python's `logging`. Tolerates repeat init.
    let _ = pyo3_log::try_init();

    m.add_function(wrap_pyfunction!(download_py, m)?)?;

    m.add_class::<PyDownloadReport>()?;
    m.add_class::<PyDownloadedFile>()?;
    m.add_class::<PyFailedFile>()?;

    m.add(
        "SimpleLocalizeError",
        m.py().get_type::<SimpleLocalizeError>(),
    )?;
    m.add("AuthError", m.py().get_type::<AuthError>())?;
    m.add("ApiError", m.py().get_type::<ApiError>())?;
    m.add("NetworkError", m.py().get_type::<NetworkError>())?;
    m.add(
        "DownloadFailedError",
        m.py().get_type::<DownloadFailedError>(),
    )?;
    Ok(())
}
