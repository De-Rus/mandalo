use crate::error::{CoreError, CoreResult};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, env: &str, key: &str) -> CoreResult<Option<String>>;
}

impl<T: SecretStore + ?Sized> SecretStore for Box<T> {
    fn get(&self, env: &str, key: &str) -> CoreResult<Option<String>> {
        (**self).get(env, key)
    }
}

/// Writing is a strictly smaller capability than reading: `EnvVarStore` and
/// `NoSecrets` can never write (a process cannot hand a variable to its future
/// selves), so widening `SecretStore` would force them to carry a method that
/// can only fail. Stores that own durable storage implement this in addition.
pub trait SecretWriter: SecretStore {
    fn set(&self, env: &str, key: &str, value: &str) -> CoreResult<()>;
    fn delete(&self, env: &str, key: &str) -> CoreResult<()>;
}

impl<T: SecretWriter + ?Sized> SecretWriter for Box<T> {
    fn set(&self, env: &str, key: &str, value: &str) -> CoreResult<()> {
        (**self).set(env, key, value)
    }

    fn delete(&self, env: &str, key: &str) -> CoreResult<()> {
        (**self).delete(env, key)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoSecrets;

impl SecretStore for NoSecrets {
    fn get(&self, _env: &str, _key: &str) -> CoreResult<Option<String>> {
        Ok(None)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EnvVarStore;

impl EnvVarStore {
    pub fn variable_name(env: &str, key: &str) -> String {
        format!("MANDALO_SECRET__{}__{}", Self::shout(env), Self::shout(key))
    }

    fn shout(raw: &str) -> String {
        raw.chars()
            .map(|c| {
                if c == '-' {
                    '_'
                } else {
                    c.to_ascii_uppercase()
                }
            })
            .collect()
    }
}

impl SecretStore for EnvVarStore {
    fn get(&self, env: &str, key: &str) -> CoreResult<Option<String>> {
        match std::env::var(Self::variable_name(env, key)) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(CoreError::Secret(format!(
                "{} is not valid UTF-8",
                Self::variable_name(env, key)
            ))),
        }
    }
}

/// A process-local secret store. The desktop app uses [`KeyringStore`]; this is
/// what tests and CI-less callers drive so no keychain is ever required.
#[derive(Debug, Default)]
pub struct MemorySecrets {
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl MemorySecrets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(pairs: &[(&str, &str, &str)]) -> Self {
        let store = Self::new();
        for (env, key, value) in pairs {
            store.set(env, key, value).expect("memory store write");
        }
        store
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, env: &str, key: &str) -> CoreResult<Option<String>> {
        Ok(self
            .values
            .lock()
            .expect("memory secret lock")
            .get(&(env.to_string(), key.to_string()))
            .cloned())
    }
}

impl SecretWriter for MemorySecrets {
    fn set(&self, env: &str, key: &str, value: &str) -> CoreResult<()> {
        self.values
            .lock()
            .expect("memory secret lock")
            .insert((env.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    fn delete(&self, env: &str, key: &str) -> CoreResult<()> {
        self.values
            .lock()
            .expect("memory secret lock")
            .remove(&(env.to_string(), key.to_string()));
        Ok(())
    }
}

/// The OS keychain, keyed by workspace **id** so moving or cloning the workspace
/// directory keeps the binding.
#[cfg(feature = "keychain")]
#[derive(Debug, Clone)]
pub struct KeyringStore {
    service: String,
    workspace_id: String,
}

#[cfg(feature = "keychain")]
pub const KEYRING_SERVICE: &str = "com.drus.mandalo";

#[cfg(feature = "keychain")]
impl KeyringStore {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        KeyringStore {
            service: KEYRING_SERVICE.to_string(),
            workspace_id: workspace_id.into(),
        }
    }

    pub fn with_service(service: impl Into<String>, workspace_id: impl Into<String>) -> Self {
        KeyringStore {
            service: service.into(),
            workspace_id: workspace_id.into(),
        }
    }

    pub fn entry_key(workspace_id: &str, env: &str, key: &str) -> String {
        format!("{workspace_id}/{env}/{key}")
    }

    fn entry(&self, env: &str, key: &str) -> CoreResult<keyring::Entry> {
        keyring::Entry::new(
            &self.service,
            &Self::entry_key(&self.workspace_id, env, key),
        )
        .map_err(|e| CoreError::Secret(format!("cannot open the keychain for {env}.{key}: {e}")))
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeyringStore {
    fn get(&self, env: &str, key: &str) -> CoreResult<Option<String>> {
        match self.entry(env, key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Secret(format!(
                "cannot read {env}.{key} from the keychain: {e}"
            ))),
        }
    }
}

#[cfg(feature = "keychain")]
impl SecretWriter for KeyringStore {
    fn set(&self, env: &str, key: &str, value: &str) -> CoreResult<()> {
        self.entry(env, key)?.set_password(value).map_err(|e| {
            CoreError::Secret(format!("cannot store {env}.{key} in the keychain: {e}"))
        })
    }

    fn delete(&self, env: &str, key: &str) -> CoreResult<()> {
        match self.entry(env, key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError::Secret(format!(
                "cannot remove {env}.{key} from the keychain: {e}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    Ask,
}

pub trait HostPolicy: Send + Sync {
    fn allow(&self, host: &str, ip: &IpAddr) -> Decision;

    /// `false` lets the runner skip DNS resolution entirely — a policy that
    /// never denies must not make the caller pay for a lookup.
    fn enforces(&self) -> bool {
        true
    }
}

impl<T: HostPolicy + ?Sized> HostPolicy for Box<T> {
    fn allow(&self, host: &str, ip: &IpAddr) -> Decision {
        (**self).allow(host, ip)
    }

    fn enforces(&self) -> bool {
        (**self).enforces()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAll;

impl HostPolicy for AllowAll {
    fn allow(&self, _host: &str, _ip: &IpAddr) -> Decision {
        Decision::Allow
    }

    fn enforces(&self) -> bool {
        false
    }
}

const METADATA_HOSTS: &[&str] = &[
    "metadata.google.internal",
    "metadata.goog",
    "metadata",
    "instance-data",
    "169.254.169.254",
    "fd00:ec2::254",
];

#[derive(Debug, Default, Clone)]
pub struct StrictPolicy {
    allowed: BTreeSet<String>,
}

impl StrictPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allowing<I, S>(hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        StrictPolicy {
            allowed: hosts
                .into_iter()
                .map(|h| h.as_ref().to_ascii_lowercase())
                .collect(),
        }
    }

    pub fn allow_host(&mut self, host: &str) {
        self.allowed.insert(host.to_ascii_lowercase());
    }
}

fn private_reason(ip: &IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            if v4.is_loopback() {
                return Some("loopback addresses are not reachable in strict mode");
            }
            if v4.is_link_local() {
                return Some("link-local addresses are not reachable in strict mode");
            }
            if v4.is_private() {
                return Some("private addresses are not reachable in strict mode");
            }
            if a == 100 && (64..128).contains(&b) {
                return Some("carrier-grade NAT addresses are not reachable in strict mode");
            }
            if v4.is_unspecified() || v4.is_broadcast() {
                return Some("this address is not a real host");
            }
            None
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return Some("loopback addresses are not reachable in strict mode");
            }
            if v6.is_unspecified() {
                return Some("this address is not a real host");
            }
            let first = v6.segments()[0];
            if first & 0xffc0 == 0xfe80 {
                return Some("link-local addresses are not reachable in strict mode");
            }
            if first & 0xfe00 == 0xfc00 {
                return Some("unique-local addresses are not reachable in strict mode");
            }
            if let Some(v4) = v6.to_ipv4_mapped() {
                return private_reason(&IpAddr::V4(v4));
            }
            None
        }
    }
}

impl HostPolicy for StrictPolicy {
    fn allow(&self, host: &str, ip: &IpAddr) -> Decision {
        let host = host.to_ascii_lowercase();
        if self.allowed.contains(&host) {
            return Decision::Allow;
        }
        if METADATA_HOSTS.contains(&host.as_str()) {
            return Decision::Deny(format!("{host} is a cloud metadata endpoint"));
        }
        if host.ends_with(".local") || host.ends_with(".internal") {
            return Decision::Deny(format!("{host} is a private network name"));
        }
        if let Some(reason) = private_reason(ip) {
            return Decision::Deny(format!("{host} resolves to {ip}: {reason}"));
        }
        Decision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn denial(policy: &StrictPolicy, host: &str, addr: &str) -> String {
        match policy.allow(host, &ip(addr)) {
            Decision::Deny(reason) => reason,
            other => panic!("expected a denial for {host} {addr}, got {other:?}"),
        }
    }

    #[test]
    fn env_var_store_uppercases_and_maps_dashes() {
        assert_eq!(
            EnvVarStore::variable_name("staging-eu", "api-token"),
            "MANDALO_SECRET__STAGING_EU__API_TOKEN"
        );
    }

    #[test]
    fn env_var_store_reads_and_misses() {
        let name = EnvVarStore::variable_name("prod", "token");
        std::env::set_var(&name, "s3cr3t");
        assert_eq!(
            EnvVarStore.get("prod", "token").unwrap(),
            Some("s3cr3t".to_string())
        );
        assert_eq!(EnvVarStore.get("prod", "absent").unwrap(), None);
        std::env::remove_var(&name);
    }

    #[test]
    fn no_secrets_never_resolves() {
        assert_eq!(NoSecrets.get("prod", "token").unwrap(), None);
    }

    #[test]
    fn memory_secrets_round_trip_and_delete() {
        let store = MemorySecrets::with(&[("prod", "token", "t0p")]);
        assert_eq!(store.get("prod", "token").unwrap(), Some("t0p".to_string()));
        assert_eq!(store.get("staging", "token").unwrap(), None);
        store.set("prod", "token", "t1p").unwrap();
        assert_eq!(store.get("prod", "token").unwrap(), Some("t1p".to_string()));
        store.delete("prod", "token").unwrap();
        assert_eq!(store.get("prod", "token").unwrap(), None);
        store.delete("prod", "token").unwrap();
    }

    #[cfg(feature = "keychain")]
    #[test]
    fn keyring_entries_are_scoped_by_workspace_id() {
        assert_eq!(
            KeyringStore::entry_key("11111111-2222-4333-8444-555555555555", "prod", "token"),
            "11111111-2222-4333-8444-555555555555/prod/token"
        );
    }

    /// The real keychain prompts and needs a session, so CI never touches it.
    /// Run with `MANDALO_KEYRING_TESTS=1 cargo test -p mandalo-core keyring_round_trip`.
    #[cfg(feature = "keychain")]
    #[test]
    fn keyring_round_trip_against_the_real_keychain() {
        if std::env::var("MANDALO_KEYRING_TESTS").as_deref() != Ok("1") {
            return;
        }
        let store =
            KeyringStore::with_service("com.drus.mandalo.test", uuid::Uuid::new_v4().to_string());
        assert_eq!(store.get("prod", "token").unwrap(), None);
        store.set("prod", "token", "keychain-value").unwrap();
        assert_eq!(
            store.get("prod", "token").unwrap(),
            Some("keychain-value".to_string())
        );
        store.delete("prod", "token").unwrap();
        assert_eq!(store.get("prod", "token").unwrap(), None);
    }

    #[test]
    fn allow_all_allows_loopback() {
        assert_eq!(
            AllowAll.allow("localhost", &ip("127.0.0.1")),
            Decision::Allow
        );
    }

    #[test]
    fn strict_denies_private_ranges() {
        let p = StrictPolicy::new();
        assert!(denial(&p, "localhost", "127.0.0.1").contains("loopback"));
        assert!(denial(&p, "box", "::1").contains("loopback"));
        assert!(denial(&p, "intranet", "10.0.0.5").contains("private"));
        assert!(denial(&p, "intranet", "172.16.3.9").contains("private"));
        assert!(denial(&p, "intranet", "192.168.1.1").contains("private"));
        assert!(denial(&p, "cgnat", "100.72.0.1").contains("carrier-grade"));
        assert!(denial(&p, "link", "169.254.10.1").contains("link-local"));
        assert!(denial(&p, "link6", "fe80::1").contains("link-local"));
        assert!(denial(&p, "ula", "fd00::1").contains("unique-local"));
    }

    #[test]
    fn strict_denies_metadata_and_local_names() {
        let p = StrictPolicy::new();
        assert!(denial(&p, "metadata.google.internal", "203.0.113.7").contains("metadata"));
        assert!(denial(&p, "169.254.169.254", "169.254.169.254").contains("metadata"));
        assert!(denial(&p, "printer.local", "203.0.113.7").contains("private network name"));
    }

    #[test]
    fn strict_allows_public_addresses() {
        let p = StrictPolicy::new();
        assert_eq!(
            p.allow("api.example.com", &ip("93.184.216.34")),
            Decision::Allow
        );
        assert_eq!(
            p.allow("api.example.com", &ip("2606:2800:220:1::1")),
            Decision::Allow
        );
    }

    #[test]
    fn strict_allows_explicitly_allowed_hosts() {
        let p = StrictPolicy::allowing(["Localhost"]);
        assert_eq!(p.allow("localhost", &ip("127.0.0.1")), Decision::Allow);
        let mut p = StrictPolicy::new();
        p.allow_host("127.0.0.1");
        assert_eq!(p.allow("127.0.0.1", &ip("127.0.0.1")), Decision::Allow);
    }

    #[test]
    fn strict_denies_ipv4_mapped_loopback() {
        let p = StrictPolicy::new();
        assert!(denial(&p, "sneaky", "::ffff:127.0.0.1").contains("loopback"));
    }
}
