//! Request/response models for the SimpleLocalize `/cli/v2/download` endpoint.

use serde::Deserialize;

/// Download format. This port intentionally supports JSON only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DownloadFormat {
    /// `single-language-json` — one nested JSON file per language + namespace.
    #[default]
    SingleLanguageJson,
}

impl DownloadFormat {
    /// The wire value sent as the `downloadFormat` query parameter.
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadFormat::SingleLanguageJson => "single-language-json",
        }
    }
}

/// Parameters for a download request. Empty collections / `None` fields are
/// omitted from the query string.
#[derive(Debug, Clone, Default)]
pub struct DownloadRequest {
    pub format: DownloadFormat,
    pub language_keys: Vec<String>,
    pub options: Vec<String>,
    pub sort: Option<String>,
    pub namespace: Option<String>,
    pub tags: Vec<String>,
    pub customer_id: Option<String>,
}

/// A single downloadable file described by the API. Unknown fields are ignored
/// and null metadata is tolerated.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadableFile {
    /// Presigned URL to fetch the file content from (no auth header needed).
    pub url: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub translation_key: Option<String>,
    #[serde(default)]
    pub remote_path: Option<String>,
}

/// Envelope returned by `GET /cli/v2/download`.
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadListResponse {
    #[serde(default)]
    pub files: Vec<DownloadableFile>,
}
