use mandalo_core::{
    bundle, collection, error::CoreError, git, git_sync, grpc, postman, request, runner, scan,
    script, workspace, AllowAll, KeyringStore,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::Manager;

type Reply<T> = Result<T, String>;

fn edge<T>(result: Result<T, CoreError>) -> Reply<T> {
    result.map_err(|e| e.to_string())
}

fn ws(workspace: &str) -> Reply<PathBuf> {
    edge(workspace::resolve_workspace(
        &edge(workspace::registry_path())?,
        workspace,
    ))
}

/// Secrets are bound to the workspace **id**, not its path, so moving or cloning
/// the directory keeps them reachable.
fn secrets(dir: &Path) -> Reply<KeyringStore> {
    let manifest = edge(workspace::read_manifest(dir))?.ok_or_else(|| {
        format!(
            "{} is not a Mándalo workspace yet — open it first",
            dir.display()
        )
    })?;
    Ok(KeyringStore::new(manifest.id))
}

#[tauri::command]
async fn send_request(spec: request::RequestSpec) -> Reply<request::ResponseData> {
    edge(request::send_request(spec).await)
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
    let run = runner::Runner::new(secrets(&dir)?, AllowAll);
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
    let run = runner::Runner::new(secrets(&dir)?, AllowAll);
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
    let store = secrets(&dir)?;
    edge(workspace::set_secret(&dir, &store, &env, &key, &value)).map(|_| ())
}

#[tauri::command]
fn clear_secret(workspace: String, env: String, key: String) -> Reply<()> {
    let dir = ws(&workspace)?;
    let store = secrets(&dir)?;
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
    let store = secrets(&dir)?;
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
    let path = edge(workspace::save_environment(&ws(&workspace)?, &env))?;
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

#[tauri::command]
fn export_bundle(workspace: String) -> Reply<bundle::ExportBundle> {
    edge(bundle::export(&ws(&workspace)?))
}

#[tauri::command]
fn import_bundle(workspace: String, json: String) -> Reply<postman::ImportReport> {
    edge(bundle::import(&ws(&workspace)?, &json))
}

#[tauri::command]
fn read_text_file_for_import(path: String) -> Reply<String> {
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(format!("import path must be absolute: {}", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[tauri::command]
fn write_text_file_for_export(path: String, contents: String) -> Reply<()> {
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(format!("export path must be absolute: {}", path.display()));
    }
    edge(workspace::atomic_write(path, &contents))
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
/// never in here — it goes straight to the keychain and `git_sync` reads it there.
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
fn git_sync(
    workspace: String,
    message: String,
    token: Option<String>,
    force: Option<bool>,
) -> Reply<git_sync::SyncOutcome> {
    let dir = ws(&workspace)?;
    let remote = edge(git_sync::status(&dir))?.remote_url.unwrap_or_default();
    let auth = git_auth(&remote, token);
    edge(git_sync::sync(
        &dir,
        &message,
        &auth,
        force.unwrap_or(false),
    ))
}

#[tauri::command]
fn git_push_branch(
    workspace: String,
    branch: String,
    message: String,
    token: Option<String>,
    force: Option<bool>,
) -> Reply<git_sync::PushedBranch> {
    let dir = ws(&workspace)?;
    let remote = edge(git_sync::status(&dir))?.remote_url.unwrap_or_default();
    let auth = git_auth(&remote, token);
    edge(git_sync::create_branch_and_push(
        &dir,
        &branch,
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
    let handle = edge(mandalo_core::stream::open(spec, std::sync::Arc::new(AllowAll), tx).await)?;
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
            export_bundle,
            import_bundle,
            read_text_file_for_import,
            write_text_file_for_export,
            default_workspace_dir,
            github_start_login,
            github_poll_login,
            github_store_pat,
            github_logout,
            github_status,
            git_status,
            git_init,
            git_clone,
            git_sync,
            git_push_branch,
            stream_open,
            stream_send,
            stream_close,
            stream_status,
            stream_list
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
