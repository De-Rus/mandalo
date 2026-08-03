pub mod bundle;
pub mod collection;
pub mod grpc;
pub mod interpolate;
pub mod postman;
pub mod request;
pub mod workspace;

use std::path::{Path, PathBuf};

#[tauri::command]
fn list_environments(workspace: String) -> Result<workspace::EnvironmentList, String> {
    workspace::list_environments(Path::new(&workspace))
}

#[tauri::command]
fn save_environment(workspace: String, env: workspace::Environment) -> Result<String, String> {
    let path = workspace::save_environment(Path::new(&workspace), &env)?;
    path.into_os_string()
        .into_string()
        .map_err(|p| format!("saved path is not valid UTF-8: {p:?}"))
}

#[tauri::command]
fn delete_environment(workspace: String, name: String) -> Result<(), String> {
    workspace::delete_environment(Path::new(&workspace), &name)
}

#[tauri::command]
fn list_requests(workspace: String) -> Result<collection::RequestList, String> {
    collection::list_requests(Path::new(&workspace))
}

#[tauri::command]
fn save_request(
    workspace: String,
    request: collection::SavedRequest,
) -> Result<String, String> {
    let path = collection::save_request(Path::new(&workspace), &request)?;
    path.into_os_string()
        .into_string()
        .map_err(|p| format!("saved path is not valid UTF-8: {p:?}"))
}

#[tauri::command]
fn delete_request(workspace: String, id: String) -> Result<(), String> {
    collection::delete_request(Path::new(&workspace), &id)
}

#[tauri::command]
fn import_postman(workspace: String, json: String) -> Result<postman::ImportReport, String> {
    postman::import(Path::new(&workspace), &json)
}

#[tauri::command]
fn export_bundle(workspace: String) -> Result<String, String> {
    bundle::export(Path::new(&workspace))
}

#[tauri::command]
fn import_bundle(workspace: String, json: String) -> Result<postman::ImportReport, String> {
    bundle::import(Path::new(&workspace), &json)
}

#[tauri::command]
fn read_text_file_for_import(path: String) -> Result<String, String> {
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(format!("import path must be absolute: {}", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[tauri::command]
fn write_text_file_for_export(path: String, contents: String) -> Result<(), String> {
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(format!("export path must be absolute: {}", path.display()));
    }
    workspace::atomic_write(path, &contents)
}

#[tauri::command]
fn default_workspace_dir() -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    PathBuf::from(home)
        .join("Mandalo")
        .into_os_string()
        .into_string()
        .map_err(|p| format!("home path is not valid UTF-8: {p:?}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            request::send_request,
            grpc::list_grpc_methods,
            grpc::send_grpc,
            list_environments,
            save_environment,
            delete_environment,
            list_requests,
            save_request,
            delete_request,
            import_postman,
            export_bundle,
            import_bundle,
            read_text_file_for_import,
            write_text_file_for_export,
            default_workspace_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
