use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum RawLanguage {
    Json,
    #[default]
    Text,
    Xml,
    Html,
    Javascript,
}

impl RawLanguage {
    pub fn content_type(self) -> &'static str {
        match self {
            RawLanguage::Json => "application/json",
            RawLanguage::Text => "text/plain",
            RawLanguage::Xml => "application/xml",
            RawLanguage::Html => "text/html",
            RawLanguage::Javascript => "application/javascript",
        }
    }

    /// Postman stores the editor language under `options.raw.language`.
    pub fn from_postman(name: &str) -> Option<RawLanguage> {
        match name {
            "json" => Some(RawLanguage::Json),
            "text" => Some(RawLanguage::Text),
            "xml" => Some(RawLanguage::Xml),
            "html" => Some(RawLanguage::Html),
            "javascript" => Some(RawLanguage::Javascript),
            _ => None,
        }
    }

    pub fn to_postman(self) -> &'static str {
        match self {
            RawLanguage::Json => "json",
            RawLanguage::Text => "text",
            RawLanguage::Xml => "xml",
            RawLanguage::Html => "html",
            RawLanguage::Javascript => "javascript",
        }
    }

    /// Only used when reading the pre-body-enum format, where a raw body was a
    /// bare string with no language recorded.
    fn sniff(text: &str) -> RawLanguage {
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return RawLanguage::Json;
        }
        if trimmed.starts_with("<?xml") {
            return RawLanguage::Xml;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
            return RawLanguage::Html;
        }
        if trimmed.starts_with('<') {
            return RawLanguage::Xml;
        }
        RawLanguage::Text
    }
}

fn enabled_default() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// One `application/x-www-form-urlencoded` field.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FormRow {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "enabled_default", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl FormRow {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> FormRow {
        FormRow {
            key: key.into(),
            value: value.into(),
            enabled: true,
            description: None,
        }
    }
}

/// One `multipart/form-data` field. A row with `files` is a file field: every
/// path becomes its own part under the same field name, which is how a browser
/// posts `<input type="file" multiple>`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FormDataRow {
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(
        default,
        alias = "file",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default = "enabled_default", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl FormDataRow {
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> FormDataRow {
        FormDataRow {
            key: key.into(),
            value: value.into(),
            files: Vec::new(),
            content_type: None,
            enabled: true,
            description: None,
        }
    }

    pub fn file(key: impl Into<String>, path: impl Into<String>) -> FormDataRow {
        FormDataRow {
            files: vec![path.into()],
            ..FormDataRow::text(key, "")
        }
    }

    pub fn files<I: IntoIterator<Item = S>, S: Into<String>>(
        key: impl Into<String>,
        paths: I,
    ) -> FormDataRow {
        FormDataRow {
            files: paths.into_iter().map(Into::into).collect(),
            ..FormDataRow::text(key, "")
        }
    }

    pub fn is_file(&self) -> bool {
        !self.files.is_empty()
    }
}

fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(path) => vec![path],
        OneOrMany::Many(paths) => paths,
    })
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(
    tag = "mode",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum Body {
    #[default]
    None,
    Raw {
        #[serde(default)]
        language: RawLanguage,
        #[serde(default)]
        text: String,
    },
    Urlencoded {
        #[serde(default)]
        rows: Vec<FormRow>,
    },
    Formdata {
        #[serde(default)]
        rows: Vec<FormDataRow>,
    },
    Binary {
        #[serde(default)]
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
    },
    Graphql {
        #[serde(default)]
        query: String,
        #[serde(default)]
        variables: String,
    },
}

impl Body {
    pub fn is_none(&self) -> bool {
        matches!(self, Body::None)
    }

    pub fn raw(text: impl Into<String>) -> Body {
        let text = text.into();
        Body::Raw {
            language: RawLanguage::sniff(&text),
            text,
        }
    }

    pub fn json(text: impl Into<String>) -> Body {
        Body::Raw {
            language: RawLanguage::Json,
            text: text.into(),
        }
    }

    pub fn graphql(query: impl Into<String>, variables: impl Into<String>) -> Body {
        Body::Graphql {
            query: query.into(),
            variables: variables.into(),
        }
    }

    /// The text a script sees as `pm.request.body.raw`, and the only shape a
    /// script may write back. Structured bodies have no single text form.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Body::Raw { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Re-applies a script's `pm.request.body` write. A script can only produce
    /// text, so it can only patch a raw body — and a raw body keeps its
    /// language. Writing text onto a structured body is refused rather than
    /// dropped: silently sending the original rows is worse than stopping.
    pub fn with_text(&self, text: Option<String>) -> CoreResult<Body> {
        Ok(match (self, text) {
            (Body::Raw { language, .. }, Some(text)) => Body::Raw {
                language: *language,
                text,
            },
            (Body::Raw { .. } | Body::None, None) => Body::None,
            (Body::None, Some(text)) => Body::raw(text),
            (other, None) => other.clone(),
            (other, Some(_)) => {
                return Err(CoreError::Script(format!(
                    "a script assigned text to pm.request.body, but this request has a {} body — a script cannot replace it",
                    other.mode()
                )))
            }
        })
    }

    pub fn mode(&self) -> &'static str {
        match self {
            Body::None => "none",
            Body::Raw { .. } => "raw",
            Body::Urlencoded { .. } => "urlencoded",
            Body::Formdata { .. } => "formdata",
            Body::Binary { .. } => "binary",
            Body::Graphql { .. } => "graphql",
        }
    }

    /// Every template this body carries, so the runner can resolve the secrets
    /// they name before the request is built.
    pub fn templates(&self) -> Vec<&str> {
        match self {
            Body::None => Vec::new(),
            Body::Raw { text, .. } => vec![text.as_str()],
            Body::Urlencoded { rows } => rows
                .iter()
                .filter(|r| r.enabled)
                .flat_map(|r| [r.key.as_str(), r.value.as_str()])
                .collect(),
            Body::Formdata { rows } => rows
                .iter()
                .filter(|r| r.enabled)
                .flat_map(|r| {
                    [r.key.as_str(), r.value.as_str()]
                        .into_iter()
                        .chain(r.files.iter().map(String::as_str))
                })
                .collect(),
            Body::Binary { file, .. } => vec![file.as_str()],
            Body::Graphql { query, variables } => vec![query.as_str(), variables.as_str()],
        }
    }
}

/// A file a body wants to send, already proven to live inside the workspace.
pub struct ResolvedFile {
    pub path: PathBuf,
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// The most a single file may weigh before Mándalo refuses to send it as a body:
/// the bytes are held in memory and copied again by the HTTP client.
pub const MAX_BODY_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Resolves a body file path against the workspace root.
///
/// A collection is shared, untrusted configuration: without this every import
/// would be able to read `~/.ssh/id_rsa` and POST it somewhere. Paths are
/// therefore workspace-relative only, and the resolved file must still be under
/// the workspace after symlinks are followed.
pub fn resolve_file(workspace: Option<&Path>, rel: &str) -> CoreResult<PathBuf> {
    resolve_workspace_file(workspace, rel, "body file")
}

/// The same resolution for any file a request points at. `what` names the kind of
/// file in every message, so a missing `proto:` never reports a missing body.
pub fn resolve_workspace_file(
    workspace: Option<&Path>,
    rel: &str,
    what: &str,
) -> CoreResult<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err(CoreError::Request(format!(
            "this {what} needs a path: pick a file inside the workspace"
        )));
    }
    let root = workspace.ok_or_else(|| {
        CoreError::Request(format!(
            "cannot read the {what} {rel:?}: this request has no workspace root, so files cannot be resolved"
        ))
    })?;
    let path = Path::new(rel);
    if path.is_absolute() || rel.starts_with('/') || rel.starts_with('\\') {
        return Err(CoreError::PathEscape(format!(
            "{what} must be a path inside the workspace, not an absolute path: {rel:?}"
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(CoreError::PathEscape(format!(
                    "{what} must stay inside the workspace: {rel:?}"
                )))
            }
        }
    }
    let target = root.join(path);
    if !target.is_file() {
        return Err(CoreError::NotFound(format!(
            "{what} not found inside the workspace: {rel:?}"
        )));
    }
    let real_root = std::fs::canonicalize(root).map_err(|e| CoreError::io(root.display(), e))?;
    let real = std::fs::canonicalize(&target).map_err(|e| CoreError::io(target.display(), e))?;
    if !real.starts_with(&real_root) {
        return Err(CoreError::PathEscape(format!(
            "{what} must stay inside the workspace: {rel:?}"
        )));
    }
    Ok(real)
}

pub fn read_file(
    workspace: Option<&Path>,
    rel: &str,
    declared_type: Option<&str>,
    vars: &BTreeMap<String, String>,
) -> CoreResult<ResolvedFile> {
    let rel = interpolate::apply(rel, vars)?;
    let path = resolve_file(workspace, &rel)?;
    let size = std::fs::metadata(&path)
        .map_err(|e| CoreError::io(path.display(), e))?
        .len();
    if size > MAX_BODY_FILE_BYTES {
        return Err(CoreError::Unsupported(format!(
            "{rel:?} is {size} bytes, past the {MAX_BODY_FILE_BYTES}-byte limit for a file Mándalo sends as a body — point the request at a smaller file"
        )));
    }
    let bytes = std::fs::read(&path).map_err(|e| CoreError::io(path.display(), e))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let content_type = declared_type
        .map(String::from)
        .unwrap_or_else(|| sniff_content_type(&path).to_string());
    Ok(ResolvedFile {
        path,
        file_name,
        content_type,
        bytes,
    })
}

pub fn sniff_content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "yaml" | "yml" => "application/yaml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

// Form encoding, not URL-path encoding: a space is `+` and everything outside
// the unreserved set is percent-encoded.
const FORM_KEEP: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'*');

pub fn encode_form_component(raw: &str) -> String {
    percent_encoding::utf8_percent_encode(raw, FORM_KEEP)
        .to_string()
        .replace("%20", "+")
}

pub fn encode_urlencoded(rows: &[FormRow], vars: &BTreeMap<String, String>) -> CoreResult<Vec<u8>> {
    let mut parts = Vec::new();
    for row in rows.iter().filter(|r| r.enabled) {
        let key = encode_form_component(&interpolate::apply(&row.key, vars)?);
        let value = encode_form_component(&interpolate::apply(&row.value, vars)?);
        parts.push(format!("{key}={value}"));
    }
    Ok(parts.join("&").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("fixtures")).unwrap();
        std::fs::write(dir.path().join("fixtures/a.png"), b"png-bytes").unwrap();
        dir
    }

    #[test]
    fn traversal_absolute_and_missing_files_are_rejected() {
        let dir = ws();
        for rel in [
            "../../../etc/passwd",
            "fixtures/../../escape.txt",
            "/etc/passwd",
            "..",
        ] {
            let err = resolve_file(Some(dir.path()), rel).unwrap_err();
            assert_eq!(err.code(), "E_PATH_ESCAPE", "{rel}");
        }
        assert_eq!(
            resolve_file(Some(dir.path()), "fixtures/ghost.png")
                .unwrap_err()
                .code(),
            "E_NOT_FOUND"
        );
        assert_eq!(
            resolve_file(Some(dir.path()), "  ").unwrap_err().code(),
            "E_REQUEST"
        );
        assert_eq!(
            resolve_file(None, "fixtures/a.png").unwrap_err().code(),
            "E_REQUEST"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_out_of_the_workspace_is_rejected() {
        let dir = ws();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"ssh-key").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            dir.path().join("fixtures/leak.txt"),
        )
        .unwrap();
        let err = resolve_file(Some(dir.path()), "fixtures/leak.txt").unwrap_err();
        assert_eq!(err.code(), "E_PATH_ESCAPE");
    }

    #[test]
    fn a_file_inside_the_workspace_resolves_with_a_sniffed_type() {
        let dir = ws();
        let file = read_file(
            Some(dir.path()),
            "fixtures/{{name}}.png",
            None,
            &BTreeMap::from([("name".to_string(), "a".to_string())]),
        )
        .unwrap();
        assert_eq!(file.file_name, "a.png");
        assert_eq!(file.content_type, "image/png");
        assert_eq!(file.bytes, b"png-bytes");
    }

    #[test]
    fn declared_content_type_wins_over_the_extension() {
        let dir = ws();
        let file = read_file(
            Some(dir.path()),
            "fixtures/a.png",
            Some("image/x-custom"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(file.content_type, "image/x-custom");
    }

    #[test]
    fn urlencoded_skips_disabled_rows_and_percent_encodes() {
        let rows = vec![
            FormRow::new("q", "a b&c=d"),
            FormRow {
                enabled: false,
                ..FormRow::new("debug", "1")
            },
            FormRow::new("name", "{{name}}"),
        ];
        let vars = BTreeMap::from([("name".to_string(), "ná va".to_string())]);
        assert_eq!(
            String::from_utf8(encode_urlencoded(&rows, &vars).unwrap()).unwrap(),
            "q=a+b%26c%3Dd&name=n%C3%A1+va"
        );
    }

    #[test]
    fn legacy_raw_text_gets_a_sniffed_language() {
        assert_eq!(RawLanguage::sniff("{\"a\": 1}"), RawLanguage::Json);
        assert_eq!(RawLanguage::sniff("  [1,2]"), RawLanguage::Json);
        assert_eq!(
            RawLanguage::sniff("<?xml version=\"1\"?>"),
            RawLanguage::Xml
        );
        assert_eq!(RawLanguage::sniff("<html><body>"), RawLanguage::Html);
        assert_eq!(RawLanguage::sniff("<user/>"), RawLanguage::Xml);
        assert_eq!(RawLanguage::sniff("hola"), RawLanguage::Text);
    }

    #[test]
    fn a_script_cannot_replace_a_structured_body_with_text() {
        let formdata = Body::Formdata {
            rows: vec![FormDataRow::file("avatar", "files/a.png")],
        };
        let err = formdata
            .with_text(Some("{}".to_string()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("formdata body"), "{err}");
        assert_eq!(formdata.with_text(None).unwrap(), formdata);
        assert_eq!(
            Body::json("{}").with_text(Some("[]".to_string())).unwrap(),
            Body::json("[]")
        );
        assert_eq!(Body::json("{}").with_text(None).unwrap(), Body::None);
    }

    #[test]
    fn a_file_row_reads_both_the_single_and_the_list_spelling() {
        let single: FormDataRow = toml::from_str("key = \"a\"\nfile = \"x.png\"").unwrap();
        let many: FormDataRow =
            toml::from_str("key = \"a\"\nfiles = [\"x.png\", \"y.png\"]").unwrap();
        assert_eq!(single.files, vec!["x.png"]);
        assert_eq!(many.files, vec!["x.png", "y.png"]);
        assert!(single.is_file());
        assert!(!FormDataRow::text("a", "b").is_file());
    }
}
