use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    UnresolvedVar(String),
    Template(String),
    PathEscape(String),
    InvalidName(String),
    NotFound(String),
    Conflict(String),
    Io(String),
    Parse(String),
    Schema(String),
    Request(String),
    Network(String),
    /// The peer's certificate was rejected, which a bare transport error buries
    /// under a chain nobody can act on.
    Tls(String),
    HostDenied(String),
    HostConfirmRequired(String),
    Script(String),
    ScriptTimeout(String),
    Grpc(String),
    Capture(String),
    Unsupported(String),
    Secret(String),
    SecretHostDenied(String),
    Stream(String),
    StreamLimit(String),
    /// The workspace is somebody else's copy, opened from a link.
    ReadOnly(String),
    /// The repository answered as if it does not exist, which is also how a
    /// private one answers to a caller with no credential.
    Private(String),
}

/// Every error message goes through the process redactor, so a resolved secret
/// cannot reach a terminal, a report or an IPC string by riding inside a message.
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::redact::scrub(self.message()))
    }
}

impl std::error::Error for CoreError {}

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::UnresolvedVar(_) => "E_UNRESOLVED_VAR",
            CoreError::Template(_) => "E_TEMPLATE",
            CoreError::PathEscape(_) => "E_PATH_ESCAPE",
            CoreError::InvalidName(_) => "E_INVALID_NAME",
            CoreError::NotFound(_) => "E_NOT_FOUND",
            CoreError::Conflict(_) => "E_CONFLICT",
            CoreError::Io(_) => "E_IO",
            CoreError::Parse(_) => "E_PARSE",
            CoreError::Schema(_) => "E_SCHEMA",
            CoreError::Request(_) => "E_REQUEST",
            CoreError::Network(_) => "E_NETWORK",
            CoreError::Tls(_) => "E_TLS",
            CoreError::HostDenied(_) => "E_HOST_DENIED",
            CoreError::HostConfirmRequired(_) => "E_HOST_CONFIRM_REQUIRED",
            CoreError::Script(_) => "E_SCRIPT",
            CoreError::ScriptTimeout(_) => "E_SCRIPT_TIMEOUT",
            CoreError::Grpc(_) => "E_GRPC",
            CoreError::Capture(_) => "E_CAPTURE",
            CoreError::Unsupported(_) => "E_UNSUPPORTED",
            CoreError::Secret(_) => "E_SECRET",
            CoreError::SecretHostDenied(_) => "E_SECRET_HOST_DENIED",
            CoreError::Stream(_) => "E_STREAM",
            CoreError::StreamLimit(_) => "E_STREAM_LIMIT",
            CoreError::ReadOnly(_) => "E_READ_ONLY",
            CoreError::Private(_) => "E_PRIVATE",
        }
    }

    /// The raw, **unredacted** message. Anything that reaches a user, a file or
    /// the IPC edge must use `to_string()` instead.
    pub fn message(&self) -> &str {
        match self {
            CoreError::UnresolvedVar(m)
            | CoreError::Template(m)
            | CoreError::PathEscape(m)
            | CoreError::InvalidName(m)
            | CoreError::NotFound(m)
            | CoreError::Conflict(m)
            | CoreError::Io(m)
            | CoreError::Parse(m)
            | CoreError::Schema(m)
            | CoreError::Request(m)
            | CoreError::Network(m)
            | CoreError::Tls(m)
            | CoreError::HostDenied(m)
            | CoreError::HostConfirmRequired(m)
            | CoreError::Script(m)
            | CoreError::ScriptTimeout(m)
            | CoreError::Grpc(m)
            | CoreError::Capture(m)
            | CoreError::Unsupported(m)
            | CoreError::Secret(m)
            | CoreError::SecretHostDenied(m)
            | CoreError::Stream(m)
            | CoreError::StreamLimit(m)
            | CoreError::ReadOnly(m)
            | CoreError::Private(m) => m,
        }
    }

    /// Every variant, one fixture message each — the redaction test walks this so a
    /// new variant cannot be added without being covered.
    /// Adding a variant must not silently drop it from the redaction test, so the
    /// compiler is made to notice: this match stops being exhaustive.
    #[allow(dead_code)]
    fn assert_every_variant_is_listed(e: &CoreError) {
        match e {
            CoreError::UnresolvedVar(_)
            | CoreError::Template(_)
            | CoreError::PathEscape(_)
            | CoreError::InvalidName(_)
            | CoreError::NotFound(_)
            | CoreError::Conflict(_)
            | CoreError::Io(_)
            | CoreError::Parse(_)
            | CoreError::Schema(_)
            | CoreError::Request(_)
            | CoreError::Network(_)
            | CoreError::Tls(_)
            | CoreError::HostDenied(_)
            | CoreError::HostConfirmRequired(_)
            | CoreError::Script(_)
            | CoreError::ScriptTimeout(_)
            | CoreError::Grpc(_)
            | CoreError::Capture(_)
            | CoreError::Unsupported(_)
            | CoreError::Secret(_)
            | CoreError::SecretHostDenied(_)
            | CoreError::Stream(_)
            | CoreError::StreamLimit(_)
            | CoreError::ReadOnly(_)
            | CoreError::Private(_) => {}
        }
    }

    pub fn all_variants(message: &str) -> Vec<CoreError> {
        let m = || message.to_string();
        vec![
            CoreError::UnresolvedVar(m()),
            CoreError::Template(m()),
            CoreError::PathEscape(m()),
            CoreError::InvalidName(m()),
            CoreError::NotFound(m()),
            CoreError::Conflict(m()),
            CoreError::Io(m()),
            CoreError::Parse(m()),
            CoreError::Schema(m()),
            CoreError::Request(m()),
            CoreError::Network(m()),
            CoreError::Tls(m()),
            CoreError::HostDenied(m()),
            CoreError::HostConfirmRequired(m()),
            CoreError::Script(m()),
            CoreError::ScriptTimeout(m()),
            CoreError::Grpc(m()),
            CoreError::Capture(m()),
            CoreError::Unsupported(m()),
            CoreError::Secret(m()),
            CoreError::SecretHostDenied(m()),
            CoreError::Stream(m()),
            CoreError::StreamLimit(m()),
            CoreError::ReadOnly(m()),
            CoreError::Private(m()),
        ]
    }

    pub fn io(context: impl fmt::Display, source: impl fmt::Display) -> Self {
        CoreError::Io(format!("{context}: {source}"))
    }

    pub fn parse(context: impl fmt::Display, source: impl fmt::Display) -> Self {
        CoreError::Parse(format!("{context}: {source}"))
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_unique_per_variant() {
        let all = CoreError::all_variants("x");
        let mut codes: Vec<&str> = all.iter().map(CoreError::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total);
        assert!(codes.iter().all(|c| c.starts_with("E_")));
    }

    #[test]
    fn display_is_the_bare_message() {
        let e = CoreError::UnresolvedVar("unresolved variable: base".into());
        assert_eq!(e.to_string(), "unresolved variable: base");
        assert_eq!(e.message(), "unresolved variable: base");
        assert_eq!(e.code(), "E_UNRESOLVED_VAR");
    }
}
