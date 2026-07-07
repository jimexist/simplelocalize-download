//! Download-path template rendering with path-traversal protection.
//!
//! Placeholders match the Java CLI: `{lang}`, `{ns}`, `{customer}`,
//! `{translationKey}`, `{remotePath}`. Missing/null metadata renders as an empty
//! string. Because placeholder values come from the server, the resolved path is
//! validated to stay within the template's literal base directory.

use std::path::{Component, Path, PathBuf};

use crate::error::Error;
use crate::model::DownloadableFile;

/// Substitute placeholders in `template` with the file's metadata.
pub fn render_template(template: &str, file: &DownloadableFile) -> String {
    template
        .replace("{lang}", file.language.as_deref().unwrap_or(""))
        .replace("{ns}", file.namespace.as_deref().unwrap_or(""))
        .replace("{customer}", file.customer.as_deref().unwrap_or(""))
        .replace(
            "{translationKey}",
            file.translation_key.as_deref().unwrap_or(""),
        )
        .replace("{remotePath}", file.remote_path.as_deref().unwrap_or(""))
}

/// Render `template` for `file` and validate the result stays inside the
/// template's literal base directory. Returns the lexically-normalized path.
pub fn resolve_output_path(template: &str, file: &DownloadableFile) -> Result<PathBuf, Error> {
    let rendered = render_template(template, file);
    if rendered.trim().is_empty() {
        return Err(Error::UnsafePath(
            "template rendered to an empty path".to_string(),
        ));
    }

    let normalized = lexical_normalize(Path::new(&rendered));

    // A leading `..` after normalization means the value escaped upward.
    if normalized
        .components()
        .next()
        .is_some_and(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::UnsafePath(format!(
            "resolved path escapes the base directory: {rendered}"
        )));
    }

    let base = base_dir(template);
    if !normalized.starts_with(&base) {
        return Err(Error::UnsafePath(format!(
            "resolved path {} is outside base directory {}",
            normalized.display(),
            base.display()
        )));
    }

    if normalized.file_name().is_none() {
        return Err(Error::UnsafePath(format!(
            "resolved path has no file name: {rendered}"
        )));
    }

    Ok(normalized)
}

/// The literal directory prefix of a template (everything before the first
/// placeholder, up to the last separator), lexically normalized.
fn base_dir(template: &str) -> PathBuf {
    let literal = match template.find('{') {
        Some(idx) => &template[..idx],
        None => template,
    };
    match literal.rfind(['/', '\\']) {
        Some(idx) => lexical_normalize(Path::new(&literal[..idx])),
        None => PathBuf::new(),
    }
}

/// Resolve `.` and `..` lexically without touching the filesystem. Leading `..`
/// components that cannot be popped are preserved (so callers can detect
/// escapes).
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(component),
            },
            other => out.push(other),
        }
    }
    out.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(language: Option<&str>, namespace: Option<&str>) -> DownloadableFile {
        DownloadableFile {
            url: "https://cdn/x".into(),
            namespace: namespace.map(str::to_string),
            language: language.map(str::to_string),
            customer: None,
            translation_key: None,
            remote_path: None,
        }
    }

    #[test]
    fn renders_lang_and_namespace() {
        let out = resolve_output_path("./json/{lang}/{ns}.json", &file(Some("en"), Some("common")))
            .unwrap();
        assert_eq!(out, PathBuf::from("json/en/common.json"));
    }

    #[test]
    fn missing_namespace_becomes_empty() {
        // Mirrors the Java CLI: an absent placeholder renders as "".
        let out = resolve_output_path("./json/{lang}/{ns}.json", &file(Some("en"), None)).unwrap();
        assert_eq!(out, PathBuf::from("json/en/.json"));
    }

    #[test]
    fn rejects_parent_traversal_in_value() {
        let err = resolve_output_path(
            "./json/{lang}/{ns}.json",
            &file(Some("en"), Some("../../etc/passwd")),
        )
        .unwrap_err();
        assert!(matches!(err, Error::UnsafePath(_)), "got {err:?}");
    }

    #[test]
    fn rejects_full_escape() {
        let err = resolve_output_path("{ns}.json", &file(None, Some("../../secret"))).unwrap_err();
        assert!(matches!(err, Error::UnsafePath(_)), "got {err:?}");
    }

    #[test]
    fn absolute_template_is_allowed() {
        let out =
            resolve_output_path("/data/{lang}/{ns}.json", &file(Some("de"), Some("app"))).unwrap();
        assert_eq!(out, PathBuf::from("/data/de/app.json"));
    }

    #[test]
    fn empty_render_rejected() {
        let err = resolve_output_path("{lang}", &file(None, None)).unwrap_err();
        assert!(matches!(err, Error::UnsafePath(_)), "got {err:?}");
    }
}
