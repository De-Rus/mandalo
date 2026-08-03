use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    value: String,
    label: String,
}

#[derive(Debug, Default, Clone)]
pub struct Redactor {
    entries: Arc<Mutex<Vec<Entry>>>,
    github: bool,
}

/// Below this, a "secret" is a word — registering it would replace innocent
/// substrings in every message and make the output unreadable, which is its own
/// way of hiding a problem. Nothing a real credential looks like is this short.
pub const MIN_REDACTABLE_LEN: usize = 6;

impl Redactor {
    pub fn new(github: bool) -> Self {
        Redactor {
            entries: Arc::new(Mutex::new(Vec::new())),
            github,
        }
    }

    pub fn register(&self, env: &str, key: &str, value: &str) {
        if value.chars().count() < MIN_REDACTABLE_LEN {
            return;
        }
        let entry = Entry {
            value: value.to_string(),
            label: format!("[redacted:{env}.{key}]"),
        };
        let mut entries = self.entries.lock().expect("redactor lock");
        if entries.iter().any(|e| e == &entry) {
            return;
        }
        entries.push(entry);
        entries.sort_by_key(|e| std::cmp::Reverse(e.value.len()));
        if self.github {
            println!("::add-mask::{value}");
        }
    }

    pub fn scrub(&self, text: &str) -> String {
        let entries = self.entries.lock().expect("redactor lock");
        let mut out = text.to_string();
        for entry in entries.iter() {
            if out.contains(&entry.value) {
                out = out.replace(&entry.value, &entry.label);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().expect("redactor lock").is_empty()
    }
}

/// Every secret the process has resolved. `CoreError`'s `Display` scrubs through it,
/// so a secret cannot reach a log, a report or an IPC error string by accident.
pub fn global() -> &'static Redactor {
    static GLOBAL: OnceLock<Redactor> = OnceLock::new();
    GLOBAL.get_or_init(|| Redactor::new(std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")))
}

pub fn register(env: &str, key: &str, value: &str) {
    global().register(env, key, value);
}

pub fn scrub(text: &str) -> String {
    global().scrub(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_registered_values_everywhere() {
        let r = Redactor::new(false);
        r.register("prod", "token", "s3cr3t-value");
        let scrubbed =
            r.scrub("Authorization: Bearer s3cr3t-value\nbody: {\"t\":\"s3cr3t-value\"}");
        assert_eq!(
            scrubbed,
            "Authorization: Bearer [redacted:prod.token]\nbody: {\"t\":\"[redacted:prod.token]\"}"
        );
        assert!(!scrubbed.contains("s3cr3t-value"));
    }

    #[test]
    fn scrubs_a_secret_echoed_back_inside_a_response_body() {
        let r = Redactor::new(false);
        r.register("staging", "api-key", "ak_live_9f8a7b6c5d4e");
        let verbose = "< HTTP/1.1 200 OK\n< body: {\"echo\":{\"key\":\"ak_live_9f8a7b6c5d4e\"}}";
        let scrubbed = r.scrub(verbose);
        assert!(!scrubbed.contains("ak_live_9f8a7b6c5d4e"));
        assert!(scrubbed.contains("[redacted:staging.api-key]"));
    }

    #[test]
    fn longer_secrets_are_replaced_before_their_prefixes() {
        let r = Redactor::new(false);
        r.register("prod", "short", "abcdef");
        r.register("prod", "long", "abcdefghij");
        assert_eq!(r.scrub("abcdefghij"), "[redacted:prod.long]");
    }

    #[test]
    fn empty_and_word_sized_values_are_never_registered() {
        let r = Redactor::new(false);
        r.register("prod", "blank", "");
        r.register("prod", "tiny", "v");
        r.register("prod", "short", "abcde");
        assert!(r.is_empty());
        assert_eq!(
            r.scrub("a value of version 5: abcde"),
            "a value of version 5: abcde"
        );
        r.register("prod", "real", "abcdef");
        assert_eq!(r.scrub("x abcdef y"), "x [redacted:prod.real] y");
    }

    #[test]
    fn the_global_redactor_is_one_instance() {
        register("global-test-env", "k", "global-fixture-secret-9f8a7b");
        assert!(!global().is_empty());
        assert_eq!(
            scrub("value=global-fixture-secret-9f8a7b"),
            "value=[redacted:global-test-env.k]"
        );
    }
}
