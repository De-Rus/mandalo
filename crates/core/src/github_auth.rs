use crate::capability::{SecretStore, SecretWriter};
use crate::error::{CoreError, CoreResult};
use crate::redact;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Mándalo's own OAuth App client id, compiled into the binary.
///
/// A client id is a **public** identifier, like Google's or Slack's: the device
/// flow has no client secret at all, so there is nothing here to protect and
/// nothing for the redactor to mask. Only the access token it earns is secret.
///
/// A fork or a self-hosted build points at its own OAuth App without patching
/// source: set `MANDALO_GITHUB_CLIENT_ID` at run time, or at build time — which
/// `option_env!` bakes in. Resolution order is argument → environment → this
/// constant. See `docs/github-auth.md`.
pub const DEFAULT_CLIENT_ID: &str = match option_env!("MANDALO_GITHUB_CLIENT_ID") {
    Some(id) => id,
    None => MANDALO_CLIENT_ID,
};

/// The OAuth App registered for the shipped desktop app and CLI.
pub const MANDALO_CLIENT_ID: &str = "Ov23liXEjTIMDJe2lT2Q";

/// Kept so a build that deliberately ships without an OAuth App still fails loud
/// rather than sending a made-up id to GitHub.
pub const CLIENT_ID_PLACEHOLDER: &str = "PASTE-THE-MANDALO-OAUTH-APP-CLIENT-ID-HERE";
pub const CLIENT_ID_ENV: &str = "MANDALO_GITHUB_CLIENT_ID";

pub const OAUTH_BASE: &str = "https://github.com";
pub const API_BASE: &str = "https://api.github.com";

/// `repo` is the narrowest **OAuth App** scope that can push to a private
/// repository: there is no read/write-code-only classic scope. `public_repo`
/// covers push to public repositories and nothing else, so it is offered
/// separately. `workflow` is deliberately absent — Mándalo pushes collection and
/// environment TOML, never `.github/workflows`, and a push that touched a
/// workflow file would be refused rather than silently granted more power.
pub const DEFAULT_SCOPES: &[&str] = &["repo"];
pub const PUBLIC_REPO_SCOPES: &[&str] = &["public_repo"];

/// The GitHub identity is per-user, not per-workspace, so it lives in its own
/// `[auth]` section of the machine-local file rather than under a workspace id.
pub const TOKEN_ENV: &str = "github";
pub const TOKEN_KEY: &str = "token";

const USER_AGENT: &str = concat!("mandalo/", env!("CARGO_PKG_VERSION"));
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// GitHub adds this many seconds when it answers `slow_down` without naming a
/// new interval. Honouring the backoff is what keeps the app off the rate limit.
const SLOW_DOWN_STEP: u64 = 5;

/// Whatever answers this flow decides how long the app waits and how often it
/// asks. Both are clamped: `Instant::now() + Duration::from_secs(u64::MAX)`
/// panics outright, and an interval of zero is a polling loop with no pause.
const MIN_INTERVAL_SECS: u64 = 1;
const MAX_INTERVAL_SECS: u64 = 300;
const MIN_EXPIRY_SECS: u64 = 30;
const MAX_EXPIRY_SECS: u64 = 60 * 60 * 24;

fn sane_interval(raw: u64) -> u64 {
    raw.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)
}

fn sane_expiry(raw: u64) -> u64 {
    raw.clamp(MIN_EXPIRY_SECS, MAX_EXPIRY_SECS)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubUser {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, alias = "avatar_url")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    Token(String),
}

/// The live half of a device flow. It is deliberately **not** serializable: the
/// device code is a bearer credential for the pending grant, so it stays on the
/// Rust side and never crosses an IPC or process boundary.
#[derive(Debug)]
pub struct DeviceHandle {
    client_id: String,
    device_code: String,
    oauth_base: String,
    interval: AtomicU64,
    expires_at: Instant,
}

impl DeviceHandle {
    pub fn interval_secs(&self) -> u64 {
        self.interval.load(Ordering::Relaxed)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs())
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    /// Never shrinks: GitHub only ever widens the floor, and a client that
    /// narrowed it again would be asking to be throttled.
    fn back_off(&self, suggested: Option<u64>) {
        let current = self.interval_secs();
        let next = match suggested {
            Some(seconds) if seconds > current => seconds,
            _ => current.saturating_add(SLOW_DOWN_STEP),
        };
        self.interval.store(sane_interval(next), Ordering::Relaxed);
    }
}

pub fn resolve_client_id(explicit: Option<&str>) -> CoreResult<String> {
    let candidate = explicit
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var(CLIENT_ID_ENV)
                .ok()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.trim().to_string());
    if candidate.is_empty() || candidate == CLIENT_ID_PLACEHOLDER {
        return Err(CoreError::Unsupported(format!(
            "this build has no GitHub OAuth App client id, so signing in to GitHub is not available — register an OAuth App with the device flow enabled and set {CLIENT_ID_ENV}, or use a personal access token instead"
        )));
    }
    Ok(candidate)
}

pub async fn start_device_flow(
    client_id: &str,
    scopes: &[&str],
) -> CoreResult<(DeviceCode, DeviceHandle)> {
    start_device_flow_at(OAUTH_BASE, client_id, scopes).await
}

pub async fn start_device_flow_at(
    oauth_base: &str,
    client_id: &str,
    scopes: &[&str],
) -> CoreResult<(DeviceCode, DeviceHandle)> {
    let client_id = resolve_client_id(Some(client_id))?;
    let scope = scopes.join(" ");
    let body = post_form(
        &format!("{oauth_base}/login/device/code"),
        &[("client_id", client_id.as_str()), ("scope", scope.as_str())],
    )
    .await?;

    if let Some(error) = body.get("error").and_then(|v| v.as_str()) {
        return Err(device_start_error(error, &body));
    }

    let device_code = string_field(&body, "device_code")?;
    let code = DeviceCode {
        user_code: string_field(&body, "user_code")?,
        verification_uri: string_field(&body, "verification_uri")?,
        expires_in: sane_expiry(u64_field(&body, "expires_in")?),
        interval: sane_interval(u64_field(&body, "interval")?),
    };
    redact::register(TOKEN_ENV, "device-code", &device_code);

    let handle = DeviceHandle {
        client_id,
        device_code,
        oauth_base: oauth_base.trim_end_matches('/').to_string(),
        interval: AtomicU64::new(code.interval),
        expires_at: Instant::now() + Duration::from_secs(code.expires_in),
    };
    Ok((code, handle))
}

/// One poll. `Pending` means "ask again after `handle.interval()`" — a `slow_down`
/// answer has already widened that interval by the time this returns.
pub async fn poll_device_flow_once(handle: &DeviceHandle) -> CoreResult<PollOutcome> {
    if handle.expired() {
        return Err(CoreError::Request(
            "the GitHub sign-in code expired before it was approved — start again".to_string(),
        ));
    }
    let body = post_form(
        &format!("{}/login/oauth/access_token", handle.oauth_base),
        &[
            ("client_id", handle.client_id.as_str()),
            ("device_code", handle.device_code.as_str()),
            ("grant_type", GRANT_TYPE),
        ],
    )
    .await?;

    if let Some(error) = body.get("error").and_then(|v| v.as_str()) {
        return match error {
            "authorization_pending" => Ok(PollOutcome::Pending),
            "slow_down" => {
                handle.back_off(body.get("interval").and_then(serde_json::Value::as_u64));
                Ok(PollOutcome::Pending)
            }
            other => Err(poll_error(other, &body)),
        };
    }

    let token = string_field(&body, "access_token")?;
    if token.is_empty() {
        return Err(CoreError::Parse(
            "GitHub answered the device flow without a token and without an error".to_string(),
        ));
    }
    redact::register(TOKEN_ENV, TOKEN_KEY, &token);
    Ok(PollOutcome::Token(token))
}

/// Polls until GitHub decides, sleeping the server-dictated interval between
/// attempts. The first sleep happens **before** the first poll: the user has not
/// even typed the code yet, so an immediate request can only burn rate limit.
pub async fn poll_device_flow(handle: &DeviceHandle) -> CoreResult<String> {
    loop {
        tokio::time::sleep(handle.interval()).await;
        match poll_device_flow_once(handle).await? {
            PollOutcome::Token(token) => return Ok(token),
            PollOutcome::Pending => continue,
        }
    }
}

pub async fn whoami(token: &str) -> CoreResult<GitHubUser> {
    whoami_at(API_BASE, token).await
}

pub async fn whoami_at(api_base: &str, token: &str) -> CoreResult<GitHubUser> {
    let token = token.trim();
    if token.is_empty() {
        return Err(CoreError::Request(
            "a GitHub token cannot be empty".to_string(),
        ));
    }
    let response = crate::request::client()?
        .get(format!("{}/user", api_base.trim_end_matches('/')))
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .header("user-agent", USER_AGENT)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| CoreError::Network(format!("cannot reach GitHub: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| CoreError::Network(format!("cannot read GitHub's answer: {e}")))?;

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(CoreError::Request(
            "GitHub rejected this token — it is wrong, expired or revoked".to_string(),
        ));
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(CoreError::Request(format!(
            "GitHub refused this token: {}",
            api_message(&text).unwrap_or_else(|| "forbidden".to_string())
        )));
    }
    if !status.is_success() {
        return Err(CoreError::Request(format!(
            "GitHub answered {} when asked who this token belongs to{}",
            status.as_u16(),
            api_message(&text)
                .map(|m| format!(": {m}"))
                .unwrap_or_default()
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| CoreError::parse("GitHub's answer to /user is not a user", e))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRepo {
    pub name: String,
    #[serde(alias = "full_name")]
    pub full_name: String,
    pub private: bool,
    #[serde(alias = "clone_url")]
    pub clone_url: String,
    #[serde(alias = "html_url")]
    pub html_url: String,
    #[serde(default, alias = "default_branch")]
    pub default_branch: String,
}

/// Repositories the signed-in user can push to, newest activity first.
/// Pages until GitHub returns an empty page or a hard cap of 300 repos.
pub async fn list_repos(token: &str) -> CoreResult<Vec<GitHubRepo>> {
    list_repos_at(API_BASE, token).await
}

pub async fn list_repos_at(api_base: &str, token: &str) -> CoreResult<Vec<GitHubRepo>> {
    let token = require_token(token)?;
    let base = api_base.trim_end_matches('/');
    let mut out = Vec::new();
    for page in 1u32..=3 {
        let url = format!(
            "{base}/user/repos?per_page=100&page={page}&sort=updated&affiliation=owner,collaborator,organization_member"
        );
        let text = github_get(&url, token).await?;
        let page_repos: Vec<GitHubRepo> = serde_json::from_str(&text)
            .map_err(|e| CoreError::parse("GitHub's answer to /user/repos is not a list", e))?;
        let n = page_repos.len();
        out.extend(page_repos);
        if n < 100 {
            break;
        }
    }
    Ok(out)
}

/// Creates a repository under the signed-in user and seeds it with a README so
/// an immediate `git clone` has a branch to check out.
pub async fn create_repo(token: &str, name: &str, private: bool) -> CoreResult<GitHubRepo> {
    create_repo_at(API_BASE, token, name, private).await
}

pub async fn create_repo_at(
    api_base: &str,
    token: &str,
    name: &str,
    private: bool,
) -> CoreResult<GitHubRepo> {
    let token = require_token(token)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Request(
            "a repository name cannot be empty".to_string(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(CoreError::Request(format!(
            "“{name}” is not a valid GitHub repository name — use letters, digits, -, _ or ."
        )));
    }
    let url = format!("{}/user/repos", api_base.trim_end_matches('/'));
    let body = serde_json::json!({
        "name": name,
        "private": private,
        "auto_init": true,
    });
    let text = github_post_json(&url, token, &body).await?;
    serde_json::from_str(&text)
        .map_err(|e| CoreError::parse("GitHub's answer to create a repo is not a repository", e))
}

fn require_token(token: &str) -> CoreResult<&str> {
    let token = token.trim();
    if token.is_empty() {
        return Err(CoreError::Request(
            "a GitHub token cannot be empty".to_string(),
        ));
    }
    Ok(token)
}

async fn github_get(url: &str, token: &str) -> CoreResult<String> {
    let response = crate::request::client()?
        .get(url)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .header("user-agent", USER_AGENT)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| CoreError::Network(format!("cannot reach GitHub: {e}")))?;
    read_github_body(response, "GET").await
}

async fn github_post_json(url: &str, token: &str, body: &serde_json::Value) -> CoreResult<String> {
    let response = crate::request::client()?
        .post(url)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .header("user-agent", USER_AGENT)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(|e| CoreError::Network(format!("cannot reach GitHub: {e}")))?;
    read_github_body(response, "POST").await
}

async fn read_github_body(response: reqwest::Response, verb: &str) -> CoreResult<String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| CoreError::Network(format!("cannot read GitHub's answer: {e}")))?;
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(CoreError::Request(
            "GitHub rejected this token — it is wrong, expired or revoked".to_string(),
        ));
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(CoreError::Request(format!(
            "GitHub refused this request: {}",
            api_message(&text).unwrap_or_else(|| "forbidden".to_string())
        )));
    }
    if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        return Err(CoreError::Request(format!(
            "GitHub could not create that repository{}",
            api_message(&text)
                .map(|m| format!(": {m}"))
                .unwrap_or_default()
        )));
    }
    if !status.is_success() {
        return Err(CoreError::Request(format!(
            "GitHub answered {} to {verb}{}",
            status.as_u16(),
            api_message(&text)
                .map(|m| format!(": {m}"))
                .unwrap_or_default()
        )));
    }
    Ok(text)
}

pub fn store_token_in(store: &dyn SecretWriter, token: &str) -> CoreResult<()> {
    let token = token.trim();
    if token.is_empty() {
        return Err(CoreError::Secret(
            "refusing to store an empty GitHub token".to_string(),
        ));
    }
    redact::register(TOKEN_ENV, TOKEN_KEY, token);
    store.set(TOKEN_ENV, TOKEN_KEY, token)
}

pub fn stored_token_in(store: &dyn SecretStore) -> CoreResult<Option<String>> {
    let found = store.get(TOKEN_ENV, TOKEN_KEY)?;
    if let Some(token) = &found {
        redact::register(TOKEN_ENV, TOKEN_KEY, token);
    }
    Ok(found)
}

pub fn clear_token_in(store: &dyn SecretWriter) -> CoreResult<()> {
    store.delete(TOKEN_ENV, TOKEN_KEY)
}

pub fn token_store() -> crate::secrets::LocalStore {
    crate::secrets::LocalStore::for_auth()
}

pub fn store_token(token: &str) -> CoreResult<()> {
    store_token_in(&token_store(), token)
}

pub fn stored_token() -> CoreResult<Option<String>> {
    stored_token_in(&token_store())
}

pub fn clear_token() -> CoreResult<()> {
    clear_token_in(&token_store())
}

async fn post_form(url: &str, form: &[(&str, &str)]) -> CoreResult<serde_json::Value> {
    let response = crate::request::client()?
        .post(url)
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .form(form)
        .send()
        .await
        .map_err(|e| CoreError::Network(format!("cannot reach GitHub: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| CoreError::Network(format!("cannot read GitHub's answer: {e}")))?;

    // Without `Accept: application/json` GitHub answers form-encoded, so a parse
    // failure here usually means a proxy or captive portal answered instead.
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        CoreError::Parse(format!(
            "GitHub answered {} with something that is not JSON — is a proxy or a captive portal in the way?",
            status.as_u16()
        ))
    })?;

    if !body.is_object() {
        return Err(CoreError::Parse(format!(
            "GitHub answered {} with a JSON value that is not an object",
            status.as_u16()
        )));
    }
    if !status.is_success() && body.get("error").is_none() {
        return Err(CoreError::Request(format!(
            "GitHub answered {} to the device flow",
            status.as_u16()
        )));
    }
    Ok(body)
}

fn string_field(body: &serde_json::Value, field: &str) -> CoreResult<String> {
    body.get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            CoreError::Parse(format!("GitHub's answer is missing a text `{field}` field"))
        })
}

fn u64_field(body: &serde_json::Value, field: &str) -> CoreResult<u64> {
    body.get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            CoreError::Parse(format!(
                "GitHub's answer is missing a whole-number `{field}` field"
            ))
        })
}

fn described(body: &serde_json::Value, fallback: &str) -> String {
    body.get("error_description")
        .and_then(serde_json::Value::as_str)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn api_message(text: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

fn device_start_error(error: &str, body: &serde_json::Value) -> CoreError {
    match error {
        "device_flow_disabled" => CoreError::Unsupported(
            "this GitHub OAuth App does not have the device flow enabled — turn it on in the app's settings".to_string(),
        ),
        "incorrect_client_credentials" | "unauthorized_client" => CoreError::Request(
            "GitHub does not recognise this OAuth App client id".to_string(),
        ),
        other => CoreError::Request(described(body, other)),
    }
}

fn poll_error(error: &str, body: &serde_json::Value) -> CoreError {
    match error {
        "expired_token" => CoreError::Request(
            "the GitHub sign-in code expired before it was approved — start again".to_string(),
        ),
        "access_denied" => {
            CoreError::Request("the GitHub sign-in was cancelled".to_string())
        }
        "incorrect_device_code" => CoreError::Request(
            "GitHub did not recognise this sign-in attempt — start again".to_string(),
        ),
        "device_flow_disabled" => CoreError::Unsupported(
            "this GitHub OAuth App does not have the device flow enabled — turn it on in the app's settings".to_string(),
        ),
        "incorrect_client_credentials" | "unauthorized_client" => CoreError::Request(
            "GitHub does not recognise this OAuth App client id".to_string(),
        ),
        "unsupported_grant_type" => CoreError::Unsupported(
            "GitHub refused the device grant type".to_string(),
        ),
        other => CoreError::Request(described(body, other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::MemorySecrets;

    fn handle(interval: u64) -> DeviceHandle {
        DeviceHandle {
            client_id: "Ov23liTESTCLIENTID".to_string(),
            device_code: "device-code-fixture".to_string(),
            oauth_base: "http://127.0.0.1:1".to_string(),
            interval: AtomicU64::new(interval),
            expires_at: Instant::now() + Duration::from_secs(900),
        }
    }

    /// One test owns `MANDALO_GITHUB_CLIENT_ID`: the process environment is shared
    /// by every test thread, so splitting this would race with itself.
    #[test]
    fn the_client_id_comes_from_argument_then_environment_then_the_build() {
        let previous = std::env::var(CLIENT_ID_ENV).ok();

        std::env::remove_var(CLIENT_ID_ENV);
        assert_eq!(resolve_client_id(None).unwrap(), MANDALO_CLIENT_ID);
        assert_eq!(
            resolve_client_id(Some(" Ov23liREAL ")).unwrap(),
            "Ov23liREAL"
        );

        std::env::set_var(CLIENT_ID_ENV, "Ov23liFROMENV");
        assert_eq!(resolve_client_id(None).unwrap(), "Ov23liFROMENV");
        assert_eq!(resolve_client_id(Some("   ")).unwrap(), "Ov23liFROMENV");
        assert_eq!(resolve_client_id(Some("Ov23liARG")).unwrap(), "Ov23liARG");

        match previous {
            Some(value) => std::env::set_var(CLIENT_ID_ENV, value),
            None => std::env::remove_var(CLIENT_ID_ENV),
        }
    }

    #[test]
    fn the_shipped_build_carries_a_registered_oauth_app() {
        assert_ne!(DEFAULT_CLIENT_ID, CLIENT_ID_PLACEHOLDER);
        assert_eq!(MANDALO_CLIENT_ID, "Ov23liXEjTIMDJe2lT2Q");
    }

    #[test]
    fn the_placeholder_is_refused_even_when_passed_explicitly() {
        let error = resolve_client_id(Some(CLIENT_ID_PLACEHOLDER)).unwrap_err();
        assert_eq!(error.code(), "E_UNSUPPORTED");
    }

    #[test]
    fn slow_down_widens_the_interval_and_never_narrows_it() {
        let h = handle(5);
        h.back_off(Some(10));
        assert_eq!(h.interval_secs(), 10);
        h.back_off(Some(7));
        assert_eq!(h.interval_secs(), 15);
        h.back_off(None);
        assert_eq!(h.interval_secs(), 20);
    }

    #[test]
    fn a_hostile_expiry_or_interval_can_neither_panic_nor_spin() {
        assert_eq!(sane_expiry(u64::MAX), MAX_EXPIRY_SECS);
        assert_eq!(sane_expiry(0), MIN_EXPIRY_SECS);
        assert_eq!(sane_expiry(900), 900);
        assert_eq!(sane_interval(0), MIN_INTERVAL_SECS);
        assert_eq!(sane_interval(u64::MAX), MAX_INTERVAL_SECS);
        assert_eq!(sane_interval(5), 5);

        let _ = Instant::now() + Duration::from_secs(sane_expiry(u64::MAX));

        let h = handle(MAX_INTERVAL_SECS);
        h.back_off(Some(u64::MAX));
        assert_eq!(h.interval_secs(), MAX_INTERVAL_SECS);
    }

    #[test]
    fn scopes_are_the_narrowest_that_can_push() {
        assert_eq!(DEFAULT_SCOPES, &["repo"]);
        assert_eq!(PUBLIC_REPO_SCOPES, &["public_repo"]);
        assert!(!DEFAULT_SCOPES.contains(&"workflow"));
        assert!(!DEFAULT_SCOPES.contains(&"admin:org"));
    }

    #[test]
    fn a_token_round_trips_through_a_store_and_can_be_cleared() {
        let store = MemorySecrets::new();
        assert_eq!(stored_token_in(&store).unwrap(), None);
        store_token_in(&store, "  gho_round_trip_fixture  ").unwrap();
        assert_eq!(
            stored_token_in(&store).unwrap(),
            Some("gho_round_trip_fixture".to_string())
        );
        clear_token_in(&store).unwrap();
        assert_eq!(stored_token_in(&store).unwrap(), None);
    }

    #[test]
    fn an_empty_token_is_never_stored() {
        let store = MemorySecrets::new();
        let error = store_token_in(&store, "   ").unwrap_err();
        assert_eq!(error.code(), "E_SECRET");
        assert_eq!(stored_token_in(&store).unwrap(), None);
    }

    #[test]
    fn storing_a_token_registers_it_with_the_redactor() {
        let store = MemorySecrets::new();
        store_token_in(&store, "gho_redactor_fixture_8f3a2b").unwrap();
        assert_eq!(
            redact::scrub("push failed with gho_redactor_fixture_8f3a2b"),
            "push failed with [redacted:github.token]"
        );
    }

    #[test]
    fn the_identity_is_stored_apart_from_every_workspace() {
        assert_eq!(TOKEN_ENV, "github");
        assert_eq!(TOKEN_KEY, "token");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.toml");
        let auth = crate::secrets::LocalStore::auth_at(&path);
        store_token_in(&auth, "gho_scope_fixture").unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("[auth.github]"));
        assert_eq!(
            crate::secrets::LocalStore::at(&path, "any-workspace")
                .get(TOKEN_ENV, TOKEN_KEY)
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn an_expired_handle_refuses_to_poll_at_all() {
        let h = DeviceHandle {
            expires_at: Instant::now() - Duration::from_secs(1),
            ..handle(5)
        };
        let error = poll_device_flow_once(&h).await.unwrap_err();
        assert_eq!(error.code(), "E_REQUEST");
        assert!(error.to_string().contains("expired"));
    }

    #[tokio::test]
    async fn an_empty_token_never_reaches_the_network() {
        let error = whoami_at("http://127.0.0.1:1", "  ").await.unwrap_err();
        assert_eq!(error.code(), "E_REQUEST");
    }

    #[tokio::test]
    async fn create_repo_rejects_a_bad_name_before_the_network() {
        let error = create_repo_at("http://127.0.0.1:1", "gho_x", "bad name!", false)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "E_REQUEST");
        assert!(error.to_string().contains("not a valid"));
    }

    #[test]
    fn every_documented_poll_error_gets_its_own_message() {
        let empty = serde_json::json!({});
        let messages: Vec<String> = [
            "expired_token",
            "access_denied",
            "incorrect_device_code",
            "unsupported_grant_type",
            "device_flow_disabled",
        ]
        .iter()
        .map(|e| poll_error(e, &empty).to_string())
        .collect();
        let mut unique = messages.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), messages.len());
        assert!(messages.iter().all(|m| !m.contains('_')));
    }
}
