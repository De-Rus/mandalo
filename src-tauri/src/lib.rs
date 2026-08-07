use mandalo_core::capability::{Decision, HostPolicy};
use mandalo_core::{
    bundle, collection, curl, error::CoreError, git, git_sync, grpc, openapi, postman, redact,
    remote, request, runner, sample, scan, script, workspace, LayeredSecrets, LocalStore,
};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use tauri::Manager;

type Reply<T> = Result<T, String>;

/// The CLI scrubs every error it prints; the desktop has to do the same, or a
/// secret echoed into a message crosses the IPC boundary in plain text.
fn edge<T>(result: Result<T, CoreError>) -> Reply<T> {
    result.map_err(|e| redact::scrub(&e.to_string()))
}

/// Hosts that are never a developer's own machine and always somebody's keys:
/// the cloud instance-metadata endpoints, reachable from any laptop that ever
/// joins a corporate VPN, and the link-local range they all live in.
const METADATA_NAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.goog",
    "metadata",
    "instance-data",
    "instance-data.ec2.internal",
];

/// Loopback and private ranges stay open — a request runner that cannot reach
/// `localhost:3000` is a request runner nobody can develop against. What is
/// closed is the set of addresses that only ever hand out credentials.
#[derive(Debug, Default, Clone, Copy)]
struct DesktopPolicy;

impl HostPolicy for DesktopPolicy {
    fn allow(&self, host: &str, ip: &IpAddr) -> Decision {
        let name = host.to_ascii_lowercase();
        if METADATA_NAMES.contains(&name.as_str()) {
            return Decision::Deny(format!("{host} is a cloud metadata endpoint"));
        }
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_link_local() {
                    return Decision::Deny(format!(
                        "{host} resolves to {ip}, the link-local range cloud metadata lives in"
                    ));
                }
                if v4.octets() == [100, 100, 100, 200] {
                    return Decision::Deny(format!("{host} resolves to {ip}, a metadata endpoint"));
                }
                if v4.is_unspecified() || v4.is_broadcast() {
                    return Decision::Deny(format!("{host} resolves to {ip}, which is not a host"));
                }
            }
            IpAddr::V6(v6) => {
                if let Some(mapped) = v6.to_ipv4_mapped() {
                    return self.allow(host, &IpAddr::V4(mapped));
                }
                if v6.segments()[0] & 0xffc0 == 0xfe80 {
                    return Decision::Deny(format!(
                        "{host} resolves to {ip}, the link-local range cloud metadata lives in"
                    ));
                }
                if v6.segments() == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254] {
                    return Decision::Deny(format!("{host} resolves to {ip}, a metadata endpoint"));
                }
                if v6.is_unspecified() {
                    return Decision::Deny(format!("{host} resolves to {ip}, which is not a host"));
                }
            }
        }
        Decision::Allow
    }
}

fn ws(workspace: &str) -> Reply<PathBuf> {
    edge(workspace::resolve_workspace(
        &edge(workspace::registry_path())?,
        workspace,
    ))
}

/// Values are bound to the workspace **id**, not its path, so moving or cloning
/// the directory keeps them reachable.
fn local_store(dir: &Path) -> Reply<LocalStore> {
    let registry = edge(workspace::registry_path())?;
    let manifest = edge(workspace::ensure_ready(&registry, dir))?;
    Ok(LocalStore::for_workspace(manifest.id))
}

/// Reading goes through the whole chain — an exported variable first, then this
/// machine's file — so the app resolves exactly what a run would.
fn secrets(dir: &Path) -> Reply<LayeredSecrets> {
    Ok(LayeredSecrets::over(local_store(dir)?))
}

#[tauri::command]
async fn send_request(spec: request::RequestSpec) -> Reply<request::ResponseData> {
    edge(request::send_request(spec, &DesktopPolicy).await)
}

#[tauri::command]
async fn list_grpc_methods(proto_paths: Vec<String>) -> Reply<Vec<grpc::GrpcMethodInfo>> {
    edge(grpc::list_grpc_methods(proto_paths).await)
}

#[tauri::command]
async fn describe_message(
    proto_paths: Vec<String>,
    type_name: String,
) -> Reply<grpc::MessageShape> {
    edge(grpc::describe_message(proto_paths, type_name).await)
}

#[tauri::command]
async fn send_grpc(spec: grpc::GrpcSpec) -> Reply<grpc::GrpcResponse> {
    edge(grpc::send_grpc(spec).await)
}

#[tauri::command]
fn execute_script(
    source: String,
    context: script::ScriptContext,
    limits: Option<script::Limits>,
) -> Reply<script::ScriptOutcome> {
    edge(script::run_script(
        &source,
        context,
        limits.unwrap_or_default(),
    ))
}

#[tauri::command]
async fn run_request_full(
    workspace: String,
    collection: String,
    path: String,
    env: Option<String>,
) -> Reply<runner::StepResult> {
    let dir = ws(&workspace)?;
    let request = edge(collection::load_request(&dir, &collection, &path))?;
    let run = runner::Runner::new(secrets(&dir)?, DesktopPolicy);
    let mut step = edge(run.run_one(&dir, &request, env.as_deref()).await)?;
    step.path = path;
    Ok(step)
}

/// The editor sends what is on screen, which may differ from what is on disk.
/// Same runner, same secret resolution, same host binding as a saved request —
/// only the source of the request differs.
#[tauri::command]
async fn run_request_draft(
    workspace: String,
    request: collection::SavedRequest,
    env: Option<String>,
) -> Reply<runner::StepResult> {
    let dir = ws(&workspace)?;
    let run = runner::Runner::new(secrets(&dir)?, DesktopPolicy);
    edge(run.run_one(&dir, &request, env.as_deref()).await)
}

#[tauri::command]
fn list_workspaces() -> Reply<workspace::WorkspaceList> {
    edge(workspace::list_workspaces(
        &edge(workspace::registry_path())?,
        &edge(workspace::default_workspace_path())?,
    ))
}

#[tauri::command]
fn create_workspace(path: String, name: String) -> Reply<workspace::WorkspaceInfo> {
    edge(workspace::create_workspace(
        &edge(workspace::registry_path())?,
        Path::new(&path),
        &name,
    ))
}

#[tauri::command]
fn open_workspace(path: String) -> Reply<workspace::WorkspaceOpen> {
    edge(workspace::open_workspace(
        &edge(workspace::registry_path())?,
        Path::new(&path),
    ))
}

#[tauri::command]
fn set_active_workspace(id: String) -> Reply<workspace::WorkspaceInfo> {
    edge(workspace::set_active_workspace(
        &edge(workspace::registry_path())?,
        &id,
    ))
}

#[tauri::command]
fn remove_workspace(id: String) -> Reply<()> {
    edge(workspace::remove_workspace(
        &edge(workspace::registry_path())?,
        &id,
    ))
}

#[tauri::command]
fn list_environments(workspace: String) -> Reply<workspace::EnvironmentViewList> {
    let dir = ws(&workspace)?;
    let store = secrets(&dir)?;
    edge(workspace::list_environment_views(&dir, &store))
}

#[tauri::command]
fn set_secret(workspace: String, env: String, key: String, value: String) -> Reply<()> {
    let dir = ws(&workspace)?;
    let store = local_store(&dir)?;
    edge(workspace::set_secret(&dir, &store, &env, &key, &value)).map(|_| ())
}

/// A value this machine keeps to itself without masking it — a URL that points
/// at the developer's own box, which must not reach the shared file either.
#[tauri::command]
fn set_local_var(workspace: String, env: String, key: String, value: String) -> Reply<()> {
    let dir = ws(&workspace)?;
    let store = local_store(&dir)?;
    edge(workspace::set_local(&dir, &store, &env, &key, &value)).map(|_| ())
}

#[tauri::command]
fn clear_secret(workspace: String, env: String, key: String) -> Reply<()> {
    let dir = ws(&workspace)?;
    let store = local_store(&dir)?;
    edge(workspace::clear_secret(&dir, &store, &env, &key))
}

#[tauri::command]
fn secret_status(workspace: String, env: String) -> Reply<BTreeMap<String, bool>> {
    let dir = ws(&workspace)?;
    let store = secrets(&dir)?;
    edge(workspace::secret_status(&dir, &store, &env))
}

#[tauri::command]
fn bind_secret_host(
    workspace: String,
    env: String,
    key: String,
    host: String,
) -> Reply<Vec<String>> {
    let dir = ws(&workspace)?;
    edge(workspace::bind_secret_host(&dir, &env, &key, &host))
}

#[tauri::command]
fn delete_var(workspace: String, env: String, key: String) -> Reply<()> {
    let dir = ws(&workspace)?;
    let store = local_store(&dir)?;
    edge(workspace::delete_var(&dir, &store, &env, &key)).map(|_| ())
}

#[tauri::command]
fn ensure_git_hygiene(workspace: String) -> Reply<git::GitHygiene> {
    edge(git::ensure_git_hygiene(&ws(&workspace)?))
}

#[tauri::command]
fn install_precommit_hook(workspace: String) -> Reply<()> {
    edge(git::install_precommit_hook(&ws(&workspace)?)).map(|_| ())
}

#[tauri::command]
fn scan_workspace(workspace: String) -> Reply<Vec<scan::Finding>> {
    edge(scan::scan_workspace(&ws(&workspace)?))
}

#[tauri::command]
fn save_environment(workspace: String, env: workspace::Environment) -> Reply<String> {
    let dir = ws(&workspace)?;
    let store = local_store(&dir)?;
    let path = edge(workspace::save_environment(&dir, &store, &env))?;
    path.into_os_string()
        .into_string()
        .map_err(|p| format!("saved path is not valid UTF-8: {p:?}"))
}

#[tauri::command]
fn delete_environment(workspace: String, name: String) -> Reply<()> {
    edge(workspace::delete_environment(&ws(&workspace)?, &name))
}

#[tauri::command]
fn list_collections(workspace: String) -> Reply<collection::CollectionList> {
    edge(collection::list_collections(&ws(&workspace)?))
}

#[tauri::command]
fn create_collection(workspace: String, name: String) -> Reply<collection::CollectionInfo> {
    edge(collection::create_collection(&ws(&workspace)?, &name))
}

#[tauri::command]
fn rename_collection(
    workspace: String,
    slug: String,
    name: String,
) -> Reply<collection::CollectionInfo> {
    edge(collection::rename_collection(
        &ws(&workspace)?,
        &slug,
        &name,
    ))
}

#[tauri::command]
fn delete_collection(workspace: String, slug: String) -> Reply<()> {
    edge(collection::delete_collection(&ws(&workspace)?, &slug))
}

#[tauri::command]
fn list_tree(workspace: String) -> Reply<collection::Tree> {
    edge(collection::list_tree(&ws(&workspace)?))
}

#[tauri::command]
fn save_request(
    workspace: String,
    collection: String,
    path: Option<String>,
    folder: Option<String>,
    request: collection::SavedRequest,
) -> Reply<collection::SavedPath> {
    edge(collection::save_request(
        &ws(&workspace)?,
        &collection,
        path.as_deref(),
        folder.as_deref(),
        &request,
    ))
}

#[tauri::command]
fn load_request(
    workspace: String,
    collection: String,
    path: String,
) -> Reply<collection::SavedRequest> {
    edge(collection::load_request(
        &ws(&workspace)?,
        &collection,
        &path,
    ))
}

#[tauri::command]
fn delete_request(workspace: String, collection: String, path: String) -> Reply<()> {
    edge(collection::delete_request(
        &ws(&workspace)?,
        &collection,
        &path,
    ))
}

#[tauri::command]
fn create_folder(workspace: String, collection: String, path: String) -> Reply<()> {
    edge(collection::create_folder(
        &ws(&workspace)?,
        &collection,
        &path,
    ))
}

#[tauri::command]
fn delete_folder(workspace: String, collection: String, path: String) -> Reply<()> {
    edge(collection::delete_folder(
        &ws(&workspace)?,
        &collection,
        &path,
    ))
}

#[tauri::command]
fn rename_folder(
    workspace: String,
    collection: String,
    path: String,
    name: String,
) -> Reply<collection::SavedPath> {
    edge(collection::rename_folder(
        &ws(&workspace)?,
        &collection,
        &path,
        &name,
    ))
}

#[tauri::command]
fn move_request(
    workspace: String,
    collection: String,
    from: String,
    to_folder: String,
) -> Reply<collection::SavedPath> {
    edge(collection::move_request(
        &ws(&workspace)?,
        &collection,
        &from,
        &to_folder,
    ))
}

#[tauri::command]
fn import_postman(workspace: String, json: String) -> Reply<postman::ImportReport> {
    edge(postman::import(&ws(&workspace)?, &json))
}

/// The whole document as text, JSON or YAML — `source` is not necessarily JSON.
#[tauri::command]
fn import_openapi(workspace: String, source: String) -> Reply<postman::ImportReport> {
    edge(openapi::import(&ws(&workspace)?, &source))
}

#[tauri::command]
fn plan_export(
    workspace: String,
    selection: Option<bundle::ExportSelection>,
    format: Option<String>,
) -> Reply<bundle::ExportPlan> {
    let format = parse_share_format(format)?;
    edge(bundle::plan_export_as(
        &ws(&workspace)?,
        &selection.unwrap_or_default(),
        format,
    ))
}

#[tauri::command]
fn run_export(
    workspace: String,
    selection: Option<bundle::ExportSelection>,
    token: String,
    path: String,
    force: Option<bool>,
    format: Option<String>,
) -> Reply<bundle::ExportReceipt> {
    let format = parse_share_format(format)?;
    edge(bundle::run_export_as(
        &ws(&workspace)?,
        &selection.unwrap_or_default(),
        format,
        &token,
        &absolute(&path)?,
        force.unwrap_or(false),
    ))
}

fn parse_share_format(
    format: Option<String>,
) -> Reply<Option<mandalo_core::workspace::ShareFormat>> {
    match format.as_deref() {
        None => Ok(None),
        Some("postman") => Ok(Some(mandalo_core::workspace::ShareFormat::Postman)),
        Some("bundle") | Some("native") => Ok(Some(mandalo_core::workspace::ShareFormat::Native)),
        Some(other) => Err(format!(
            "unknown export format {other:?} — use bundle or postman"
        )),
    }
}

#[tauri::command]
fn workspace_share(workspace: String) -> Reply<Option<mandalo_core::workspace::ShareConfig>> {
    edge(mandalo_core::workspace::share_config(&ws(&workspace)?))
}

#[tauri::command]
fn set_workspace_share(
    workspace: String,
    format: Option<String>,
    dir: Option<String>,
) -> Reply<mandalo_core::workspace::Manifest> {
    let share = match format.as_deref() {
        None | Some("native") | Some("bundle") => None,
        Some("postman") => Some(mandalo_core::workspace::ShareConfig {
            format: mandalo_core::workspace::ShareFormat::Postman,
            dir: dir.filter(|d| !d.trim().is_empty()),
        }),
        Some(other) => {
            return Err(format!(
                "unknown share format {other:?} — use native or postman"
            ));
        }
    };
    edge(mandalo_core::workspace::set_share(&ws(&workspace)?, share))
}

#[tauri::command]
fn import_bundle(workspace: String, json: String) -> Reply<postman::ImportReport> {
    edge(bundle::import(&ws(&workspace)?, &json))
}

/// The only files these two commands may touch. They exist to carry a collection
/// in and out through a file picker, so they are held to the shapes a collection
/// arrives in — and never to a dotfile, which is where a machine keeps its keys
/// and its shell startup.
const TRANSFER_EXTENSIONS: &[&str] = &[
    "json", "toml", "http", "grpc", "ws", "mqtt", "yaml", "yml", "md", "txt",
];

const MAX_TRANSFER_BYTES: u64 = request::MAX_IMPORT_BYTES as u64;

fn transfer_path(raw: &str, what: &str) -> Reply<PathBuf> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(format!("{what} path must be absolute: {}", path.display()));
    }
    if path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|part| part.starts_with('.') && part != "." && part != "..")
    {
        return Err(format!(
            "{} is a hidden file or lives in a hidden directory, which {what} cannot touch",
            path.display()
        ));
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !TRANSFER_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            "{what} works with {} files, and {} is not one",
            TRANSFER_EXTENSIONS.join(", "),
            path.display()
        ));
    }
    Ok(path)
}

#[tauri::command]
fn read_text_file_for_import(path: String) -> Reply<String> {
    let path = transfer_path(&path, "an import")?;
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > MAX_TRANSFER_BYTES {
        return Err(format!(
            "{} is {size} bytes, over the {MAX_TRANSFER_BYTES} byte limit for an import",
            path.display()
        ));
    }
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[tauri::command]
fn write_text_file_for_export(path: String, contents: String) -> Reply<()> {
    let path = transfer_path(&path, "an export")?;
    edge(workspace::atomic_write(&path, &contents))
}

#[tauri::command]
fn default_workspace_dir() -> Reply<String> {
    edge(workspace::default_workspace_path())?
        .into_os_string()
        .into_string()
        .map_err(|p| format!("home path is not valid UTF-8: {p:?}"))
}

/// Device flows in progress. The `DeviceHandle` — and the device code inside it —
/// stays here: the frontend only ever holds the opaque key, so no credential of
/// any kind crosses the IPC boundary in either direction.
#[derive(Default)]
struct GithubFlows(
    std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<mandalo_core::github_auth::DeviceHandle>>,
    >,
);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GithubLoginStart {
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
    handle: String,
}

/// `pending` until GitHub says otherwise, then the identity. The token itself is
/// never in here — it goes straight to `secrets.toml` and `git_sync` reads it there.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GithubPoll {
    pending: bool,
    user: Option<mandalo_core::github_auth::GitHubUser>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GithubStatus {
    connected: bool,
    login: Option<String>,
}

#[tauri::command]
async fn github_start_login(
    flows: tauri::State<'_, GithubFlows>,
    public_only: Option<bool>,
    client_id: Option<String>,
) -> Reply<GithubLoginStart> {
    use mandalo_core::github_auth as gh;
    let id = edge(gh::resolve_client_id(client_id.as_deref()))?;
    let scopes = if public_only.unwrap_or(false) {
        gh::PUBLIC_REPO_SCOPES
    } else {
        gh::DEFAULT_SCOPES
    };
    let (code, handle) = edge(gh::start_device_flow(&id, scopes).await)?;
    let key = uuid::Uuid::new_v4().to_string();
    flows
        .0
        .lock()
        .map_err(|_| "the GitHub sign-in state is poisoned".to_string())?
        .insert(key.clone(), std::sync::Arc::new(handle));
    Ok(GithubLoginStart {
        user_code: code.user_code,
        verification_uri: code.verification_uri,
        expires_in: code.expires_in,
        interval: code.interval,
        handle: key,
    })
}

#[tauri::command]
async fn github_poll_login(
    flows: tauri::State<'_, GithubFlows>,
    handle: String,
) -> Reply<GithubPoll> {
    use mandalo_core::github_auth as gh;
    let flow = flows
        .0
        .lock()
        .map_err(|_| "the GitHub sign-in state is poisoned".to_string())?
        .get(&handle)
        .cloned()
        .ok_or_else(|| "this GitHub sign-in is no longer running — start again".to_string())?;

    let outcome = match gh::poll_device_flow_once(&flow).await {
        Ok(outcome) => outcome,
        Err(e) => {
            forget_flow(&flows, &handle);
            return Err(e.to_string());
        }
    };
    match outcome {
        gh::PollOutcome::Pending => Ok(GithubPoll {
            pending: true,
            user: None,
        }),
        gh::PollOutcome::Token(token) => {
            forget_flow(&flows, &handle);
            let user = edge(gh::whoami(&token).await)?;
            edge(gh::store_token(&token))?;
            Ok(GithubPoll {
                pending: false,
                user: Some(user),
            })
        }
    }
}

fn forget_flow(flows: &tauri::State<'_, GithubFlows>, handle: &str) {
    if let Ok(mut open) = flows.0.lock() {
        open.remove(handle);
    }
}

/// The token is validated against GitHub before it is stored, so a mistyped or
/// revoked paste fails immediately instead of at the next push.
#[tauri::command]
async fn github_store_pat(token: String) -> Reply<mandalo_core::github_auth::GitHubUser> {
    use mandalo_core::github_auth as gh;
    let user = edge(gh::whoami(&token).await)?;
    edge(gh::store_token(&token))?;
    Ok(user)
}

#[tauri::command]
fn github_logout() -> Reply<()> {
    edge(mandalo_core::github_auth::clear_token())
}

#[tauri::command]
async fn github_status() -> Reply<GithubStatus> {
    use mandalo_core::github_auth as gh;
    let Some(token) = edge(gh::stored_token())? else {
        return Ok(GithubStatus {
            connected: false,
            login: None,
        });
    };
    let user = edge(gh::whoami(&token).await)?;
    Ok(GithubStatus {
        connected: true,
        login: Some(user.login),
    })
}

#[tauri::command]
async fn github_list_repos() -> Reply<Vec<mandalo_core::github_auth::GitHubRepo>> {
    use mandalo_core::github_auth as gh;
    let token = edge(gh::stored_token())?.ok_or_else(|| {
        "sign in to GitHub first — use Clone from GitHub in the workspace menu".to_string()
    })?;
    edge(gh::list_repos(&token).await)
}

#[tauri::command]
async fn github_create_repo(
    name: String,
    private: Option<bool>,
) -> Reply<mandalo_core::github_auth::GitHubRepo> {
    use mandalo_core::github_auth as gh;
    let token = edge(gh::stored_token())?.ok_or_else(|| {
        "sign in to GitHub first — use Clone from GitHub in the workspace menu".to_string()
    })?;
    edge(gh::create_repo(&token, &name, private.unwrap_or(true)).await)
}

/// The token the user signed in with, unless the caller handed one over for this
/// call. Core never reads it ambiently — resolving it is this edge's job.
fn git_auth(url: &str, token: Option<String>) -> git_sync::Auth {
    let resolved = token
        .filter(|t| !t.trim().is_empty())
        .or_else(|| mandalo_core::github_auth::stored_token().ok().flatten());
    git_sync::Auth::for_url(url, resolved.as_deref())
}

fn absolute(path: &str) -> Reply<PathBuf> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("path must be absolute: {}", path.display()));
    }
    Ok(path)
}

#[tauri::command]
fn git_status(workspace: String) -> Reply<git_sync::SyncStatus> {
    edge(git_sync::status(&ws(&workspace)?))
}

/// Same variable frame as Send, so the printed command carries exactly what
/// would have gone on the wire — an unresolved `{{var}}` fails here rather than
/// being pasted into a shell.
#[tauri::command]
fn to_curl(
    workspace: String,
    request: collection::SavedRequest,
    env: Option<String>,
) -> Reply<String> {
    let dir = ws(&workspace)?;
    let store = secrets(&dir)?;
    let frame = edge(runner::env_frame(&dir, &store, env.as_deref()))?;
    let spec = request::RequestSpec {
        kind: request.kind.clone(),
        method: request.method.clone(),
        url: request.url.clone(),
        headers: request.headers.clone(),
        body: request.body.clone(),
        auth: request.auth.clone(),
        vars: frame.vars.clone(),
        workspace: Some(dir),
    };
    edge(curl::to_curl(&spec))
}

#[tauri::command]
fn git_init(workspace: String, remote_url: Option<String>) -> Reply<()> {
    edge(git_sync::init(&ws(&workspace)?, remote_url.as_deref()))
}

#[tauri::command]
fn git_clone(url: String, dest: String, token: Option<String>) -> Reply<()> {
    let auth = git_auth(&url, token);
    edge(git_sync::clone(&url, &absolute(&dest)?, &auth))
}

#[tauri::command]
fn plan_sync(
    workspace: String,
    selection: Option<git_sync::SyncSelection>,
    branch: Option<String>,
) -> Reply<git_sync::SyncPlan> {
    edge(git_sync::plan_sync(
        &ws(&workspace)?,
        &selection.unwrap_or_default(),
        branch.as_deref(),
    ))
}

#[tauri::command]
fn run_sync(
    workspace: String,
    selection: Option<git_sync::SyncSelection>,
    token: String,
    message: String,
    git_token: Option<String>,
    force: Option<bool>,
) -> Reply<git_sync::SyncOutcome> {
    let dir = ws(&workspace)?;
    let remote = edge(git_sync::status(&dir))?.remote_url.unwrap_or_default();
    let auth = git_auth(&remote, git_token);
    edge(git_sync::run_sync(
        &dir,
        &selection.unwrap_or_default(),
        &token,
        &message,
        &auth,
        force.unwrap_or(false),
    ))
}

#[tauri::command]
fn conflict_previews(workspace: String, files: Vec<String>) -> Reply<Vec<git_sync::ConflictItem>> {
    edge(git_sync::conflict_previews(&ws(&workspace)?, &files))
}

#[tauri::command]
fn apply_conflict_choices(
    workspace: String,
    decisions: Vec<git_sync::ConflictDecision>,
) -> Reply<()> {
    edge(git_sync::apply_conflict_choices(
        &ws(&workspace)?,
        &decisions,
    ))
}

#[tauri::command]
fn run_branch_push(
    workspace: String,
    selection: Option<git_sync::SyncSelection>,
    branch: String,
    token: String,
    message: String,
    git_token: Option<String>,
    force: Option<bool>,
) -> Reply<git_sync::PushedBranch> {
    let dir = ws(&workspace)?;
    let remote = edge(git_sync::status(&dir))?.remote_url.unwrap_or_default();
    let auth = git_auth(&remote, git_token);
    edge(git_sync::run_branch_push(
        &dir,
        &selection.unwrap_or_default(),
        &branch,
        &token,
        &message,
        &auth,
        force.unwrap_or(false),
    ))
}

/// Every realtime stream this window opened. A stream outlives the invoke that
/// created it, so the handles live here and are closed on `stream_close` and on
/// window destruction — a reloaded or closed webview never leaks a socket.
#[derive(Default)]
struct Streams(mandalo_core::stream::StreamRegistry);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamOpened {
    stream_id: String,
}

/// The events go through a typed per-invocation `Channel` rather than a global
/// `emit`: the frontend cannot receive another stream's traffic by mistake, and
/// when the receiving side is dropped the send fails and the stream is closed.
#[tauri::command]
async fn stream_open(
    app: tauri::AppHandle,
    streams: tauri::State<'_, Streams>,
    spec: mandalo_core::stream::StreamSpec,
    channel: tauri::ipc::Channel<mandalo_core::stream::StreamEvent>,
) -> Reply<StreamOpened> {
    let (tx, mut events) = mandalo_core::stream::event_channel(&spec.limits);
    let handle =
        edge(mandalo_core::stream::open(spec, std::sync::Arc::new(DesktopPolicy), tx).await)?;
    let stream_id = streams.0.insert(handle);

    let closing = stream_id.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            if channel.send(event).is_err() {
                break;
            }
        }
        let _ = app.state::<Streams>().0.close(&closing).await;
    });

    Ok(StreamOpened { stream_id })
}

/// The connection a saved request describes, with the environment's variables
/// already on it. The path is resolved and confined here, so the window never
/// hands a url straight to the socket.
#[tauri::command]
fn stream_spec(
    workspace: String,
    collection: String,
    path: String,
    env: Option<String>,
) -> Reply<mandalo_core::stream::StreamSpec> {
    let dir = ws(&workspace)?;
    let request = edge(collection::load_request(&dir, &collection, &path))?;
    let frame = edge(runner::env_frame(&dir, &secrets(&dir)?, env.as_deref()))?;
    edge(mandalo_core::stream::spec_for(&request, frame.vars))
}

/// One named message of a saved stream, as the outgoing frame its protocol
/// needs — the window sends it by name, not by rebuilding the payload.
#[tauri::command]
fn stream_message(
    workspace: String,
    collection: String,
    path: String,
    name: String,
    env: Option<String>,
) -> Reply<mandalo_core::stream::Outgoing> {
    let dir = ws(&workspace)?;
    let request = edge(collection::load_request(&dir, &collection, &path))?;
    let stream = request
        .stream
        .as_ref()
        .ok_or_else(|| format!("{path} is a {} request, not a stream", request.kind))?;
    let frame = edge(runner::env_frame(&dir, &secrets(&dir)?, env.as_deref()))?;
    edge(mandalo_core::stream::outgoing_for(
        stream,
        &name,
        &frame.vars,
    ))
}

#[tauri::command]
async fn stream_send(
    streams: tauri::State<'_, Streams>,
    stream_id: String,
    payload: mandalo_core::stream::Outgoing,
) -> Reply<()> {
    edge(streams.0.send(&stream_id, payload).await)
}

#[tauri::command]
async fn stream_close(streams: tauri::State<'_, Streams>, stream_id: String) -> Reply<()> {
    edge(streams.0.close(&stream_id).await)
}

#[tauri::command]
fn stream_status(
    streams: tauri::State<'_, Streams>,
    stream_id: String,
) -> Reply<mandalo_core::stream::StreamStatus> {
    edge(streams.0.status(&stream_id))
}

#[tauri::command]
fn stream_list(
    streams: tauri::State<'_, Streams>,
) -> Reply<Vec<mandalo_core::stream::StreamStatus>> {
    Ok(streams
        .0
        .ids()
        .iter()
        .filter_map(|id| streams.0.status(id).ok())
        .collect())
}

/// The webview must not fetch an import url itself: it would reach whatever the
/// user typed with none of the egress policy the send path is held to.
#[tauri::command]
async fn fetch_text_for_import(url: String) -> Reply<request::FetchedDocument> {
    edge(request::fetch_document(&url, &DesktopPolicy).await)
}

/// A remote collection that has been read but not opened. The review the user is
/// looking at describes exactly these bytes, so the adoption writes them rather
/// than fetching again — a second fetch could return something else.
static PENDING_REMOTE: std::sync::OnceLock<
    std::sync::Mutex<BTreeMap<String, mandalo_core::remote::RemoteFetch>>,
> = std::sync::OnceLock::new();

fn pending_remote() -> &'static std::sync::Mutex<BTreeMap<String, remote::RemoteFetch>> {
    PENDING_REMOTE.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

#[tauri::command]
fn add_sample_collection(workspace: String) -> Reply<String> {
    edge(sample::add_sample_collection(&ws(&workspace)?))
}

#[tauri::command]
fn seed_workspace(workspace: String) -> Reply<Option<postman::ImportReport>> {
    let dir = ws(&workspace)?;
    let registry = edge(workspace::registry_path())?;
    edge(workspace::ensure_ready(&registry, &dir))?;
    edge(openapi::seed_if_empty(&dir))
}

/// Reads a published collection and says what it is. Nothing is written and
/// nothing runs: the files are fetched through the same egress guard as any
/// request, parsed, and described.
#[tauri::command]
async fn review_remote(source: String) -> Reply<remote::RemoteReview> {
    let parsed = edge(remote::parse_source(&source))?;
    let fetched = edge(
        remote::fetch(&parsed, &remote::Endpoints::github(), &DesktopPolicy)
            .await
            .map_err(|e| match e {
                CoreError::Private(message) => {
                    CoreError::Private(match remote::clone_url(&parsed) {
                        Some(url) => format!("{message} Sign in to GitHub, then clone {url}."),
                        None => message,
                    })
                }
                other => other,
            }),
    )?;
    let review = edge(remote::review(&fetched))?;
    let mut held = pending_remote()
        .lock()
        .map_err(|_| "the remote collection could not be held for review".to_string())?;
    // Each of these is bounded by the fetch caps, but a user who looks at one
    // repository after another must not accumulate them all.
    while held.len() >= 4 {
        let oldest = held.keys().next().cloned().unwrap_or_default();
        held.remove(&oldest);
    }
    held.insert(review.token.clone(), fetched);
    Ok(review)
}

/// Writes the reviewed collection into a read-only workspace of its own.
#[tauri::command]
fn adopt_remote(token: String, dest: String) -> Reply<workspace::WorkspaceInfo> {
    let fetched = pending_remote()
        .lock()
        .map_err(|_| "the remote collection could not be read back".to_string())?
        .remove(&token)
        .ok_or_else(|| {
            redact::scrub(&mandalo_core::review::stale("remote collection").to_string())
        })?;
    edge(remote::adopt(
        &fetched,
        &token,
        &edge(workspace::registry_path())?,
        &absolute(&dest)?,
    ))
}

/// Where this workspace came from, or `null` for one the user owns. The shell
/// asks on every switch: a read-only workspace has to look like one.
#[tauri::command]
fn remote_origin(workspace: String) -> Reply<Option<remote::RemoteOrigin>> {
    edge(remote::origin(&ws(&workspace)?))
}

#[tauri::command]
fn save_workspace_copy(
    workspace: String,
    dest: String,
    name: String,
) -> Reply<workspace::WorkspaceInfo> {
    edge(remote::save_copy(
        &ws(&workspace)?,
        &edge(workspace::registry_path())?,
        &absolute(&dest)?,
        &name,
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.webview_windows().values().next() {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(GithubFlows::default())
        .manage(Streams::default())
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let streams = window.state::<Streams>();
                tauri::async_runtime::block_on(streams.0.close_all());
            }
        })
        .invoke_handler(tauri::generate_handler![
            send_request,
            list_grpc_methods,
            describe_message,
            send_grpc,
            execute_script,
            run_request_full,
            run_request_draft,
            list_workspaces,
            create_workspace,
            open_workspace,
            set_active_workspace,
            remove_workspace,
            list_environments,
            save_environment,
            delete_environment,
            set_secret,
            set_local_var,
            clear_secret,
            secret_status,
            bind_secret_host,
            delete_var,
            ensure_git_hygiene,
            install_precommit_hook,
            scan_workspace,
            list_collections,
            create_collection,
            rename_collection,
            delete_collection,
            list_tree,
            save_request,
            load_request,
            delete_request,
            create_folder,
            delete_folder,
            rename_folder,
            move_request,
            import_postman,
            import_openapi,
            plan_export,
            run_export,
            workspace_share,
            set_workspace_share,
            import_bundle,
            read_text_file_for_import,
            fetch_text_for_import,
            write_text_file_for_export,
            default_workspace_dir,
            github_start_login,
            github_poll_login,
            github_store_pat,
            github_logout,
            github_status,
            github_list_repos,
            github_create_repo,
            git_status,
            git_init,
            git_clone,
            to_curl,
            plan_sync,
            run_sync,
            conflict_previews,
            apply_conflict_choices,
            run_branch_push,
            stream_open,
            stream_spec,
            stream_message,
            stream_send,
            stream_close,
            stream_status,
            stream_list,
            add_sample_collection,
            seed_workspace,
            review_remote,
            adopt_remote,
            remote_origin,
            save_workspace_copy
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn denied(host: &str, addr: &str) -> String {
        match DesktopPolicy.allow(host, &ip(addr)) {
            Decision::Deny(reason) => reason,
            other => panic!("expected {host} {addr} to be denied, got {other:?}"),
        }
    }

    #[test]
    fn the_desktop_blocks_metadata_and_link_local_and_nothing_else() {
        assert!(denied("169.254.169.254", "169.254.169.254").contains("link-local"));
        assert!(denied("metadata.google.internal", "203.0.113.7").contains("metadata"));
        assert!(denied("metadata", "203.0.113.7").contains("metadata"));
        assert!(denied("alibaba", "100.100.100.200").contains("metadata"));
        assert!(denied("ec2", "fd00:ec2::254").contains("metadata"));
        assert!(denied("sneaky", "::ffff:169.254.169.254").contains("link-local"));
        assert!(denied("nowhere", "0.0.0.0").contains("not a host"));

        for (host, addr) in [
            ("localhost", "127.0.0.1"),
            ("localhost", "::1"),
            ("dev-box", "192.168.1.10"),
            ("intranet", "10.0.0.5"),
            ("api.acme.com", "93.184.216.34"),
        ] {
            assert_eq!(
                DesktopPolicy.allow(host, &ip(addr)),
                Decision::Allow,
                "{host} {addr} has to stay reachable"
            );
        }
    }

    #[test]
    fn the_file_transfer_commands_refuse_anything_that_is_not_a_collection_file() {
        for bad in [
            "/Users/someone/.zshrc",
            "/Users/someone/.ssh/id_rsa",
            "/Users/someone/.aws/credentials",
            "/etc/passwd",
            "/Users/someone/.config/mandalo/secrets.toml",
            "/Users/someone/notes",
        ] {
            assert!(transfer_path(bad, "an export").is_err(), "{bad}");
        }
        assert!(transfer_path("collections/api.http", "an import").is_err());
        assert!(transfer_path("/Users/someone/Downloads/api.json", "an import").is_ok());
        assert!(transfer_path("/Users/someone/Downloads/bundle.toml", "an export").is_ok());
    }
}
