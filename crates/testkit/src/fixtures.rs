use mandalo_core::assertions::Scripts;
use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::request::Auth;
use mandalo_core::runner::Runner;
use mandalo_core::workspace::{self, Environment, Manifest, SCHEMA_VERSION};
use mandalo_core::{AllowAll, Body, NoSecrets};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn request(name: &str, method: &str, url: &str) -> SavedRequest {
    SavedRequest {
        id: name.to_ascii_lowercase().replace(' ', "-"),
        name: name.to_string(),
        kind: "http".to_string(),
        method: method.to_string(),
        url: url.to_string(),
        description: None,
        headers: Vec::new(),
        auth: Auth::None,
        body: Body::None,
        grpc: None,
        stream: None,
        scripts: Scripts::default(),
        tests: Vec::new(),
        captures: Vec::new(),
    }
}

pub fn runner() -> Runner<NoSecrets, AllowAll> {
    Runner::new(NoSecrets, AllowAll)
}

/// Creates a workspace with one empty collection and returns its slug.
pub fn workspace(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("environments")).expect("environments directory");
    std::fs::create_dir_all(dir.join("collections")).expect("collections directory");
    workspace::write_manifest(
        dir,
        &Manifest {
            schema_version: SCHEMA_VERSION,
            id: "testkit".to_string(),
            name: "Testkit".to_string(),
            remote: None,
            share: None,
        },
    )
    .expect("workspace manifest");
    collection::ensure_collection(dir, "api", "API", None)
        .expect("collection")
        .slug
}

pub fn environment(dir: &Path, name: &str, vars: &[(&str, &str)]) {
    let vars: BTreeMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    workspace::save_environment(
        dir,
        &mandalo_core::capability::RefuseLocalWrites,
        &Environment {
            name: name.to_string(),
            vars,
        },
    )
    .expect("environment");
}

pub fn example_workspace_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mock-workspace")
}

/// Copies the shipped `examples/mock-workspace` fixture into `dir` and points its
/// `local` environment at a running mock, so a test drives the same files a user opens.
pub fn install_example_workspace(dir: &Path, base_url: &str, proto_url: &str, proto_path: &str) {
    copy_tree(&example_workspace_source(), dir);
    environment(
        dir,
        "local",
        &[
            ("baseUrl", base_url),
            ("grpcUrl", proto_url),
            ("protoPath", proto_path),
        ],
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("copy target");
    for entry in std::fs::read_dir(from).expect("read fixture") {
        let entry = entry.expect("fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}
