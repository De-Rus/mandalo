use crate::error::{CoreError, CoreResult};
use git2::{ObjectType, Oid};

const UNIT: char = '\u{1e}';

/// A digest of exactly what a preview showed. Every `run_*` recomputes the plan
/// and refuses to act when the token no longer matches, so a caller cannot
/// review one thing and then have another thing happen.
pub fn token(kind: &str, parts: &[String]) -> CoreResult<String> {
    let mut payload = String::from(kind);
    for part in parts {
        payload.push(UNIT);
        payload.push_str(part);
    }
    Oid::hash_object(ObjectType::Blob, payload.as_bytes())
        .map(|oid| oid.to_string())
        .map_err(|e| CoreError::Io(format!("cannot derive a review token: {}", e.message())))
}

pub fn stale(what: &str) -> CoreError {
    CoreError::Conflict(format!(
        "the workspace changed since this {what} was reviewed — look at it again before it runs"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_changes_with_every_part_and_with_the_kind() {
        let a = token("export", &["one".to_string(), "two".to_string()]).unwrap();
        assert_eq!(
            a,
            token("export", &["one".to_string(), "two".to_string()]).unwrap()
        );
        assert_ne!(
            a,
            token("export", &["one".to_string(), "three".to_string()]).unwrap()
        );
        assert_ne!(
            a,
            token("sync", &["one".to_string(), "two".to_string()]).unwrap()
        );
    }

    #[test]
    fn parts_cannot_be_shuffled_across_the_separator() {
        assert_ne!(
            token("export", &["a".to_string(), "b".to_string()]).unwrap(),
            token("export", &["ab".to_string()]).unwrap()
        );
    }
}
