use mandalo_core::collection;
use mandalo_core::runner::VarFrame;
use mandalo_testkit::fixtures::{runner, workspace};
use mandalo_testkit::MockApi;
use std::path::Path;

fn write(ws: &Path, slug: &str, rel: &str, source: &str) {
    let path = ws.join("collections").join(slug).join(rel);
    std::fs::create_dir_all(path.parent().expect("a parent directory")).expect("the folder");
    std::fs::write(&path, source).expect("the request file");
}

/// Everything a `.http` file describes has to reach the wire unchanged, and the
/// script attached to it has to run in the same `pm.*` engine a TOML request used.
#[tokio::test]
async fn a_http_file_sends_exactly_what_its_text_describes() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;

    write(
        ws,
        &slug,
        "api.http",
        &format!(
            "\
@tenant = acme

### Post something
POST {}
Content-Type: application/json
X-Tenant: {{{{tenant}}}}
Authorization: Bearer {{{{token}}}}

{{\"hello\": \"mock\", \"tenant\": \"{{{{tenant}}}}\"}}

> {{%
pm.environment.set(\"echoed\", pm.response.json().method);
pm.test(\"the mock answered\", () => pm.response.to.have.status(200));
%}}

### Read it back
GET {}
Accept: application/json
",
            api.url("/post"),
            api.url("/get?page=2")
        ),
    );

    let tree = collection::list_tree(ws).unwrap();
    let node = &tree.collections[0];
    assert!(tree.skipped.is_empty(), "{:?}", tree.skipped);
    let paths: Vec<&str> = node.requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, vec!["api.http#0", "api.http#1"]);
    assert_eq!(node.requests[0].name, "Post something");
    assert_eq!(node.requests[0].method, "POST");
    assert_eq!(node.requests[1].method, "GET");

    let request = collection::load_request(ws, &slug, "api.http#0").unwrap();
    let mut vars = VarFrame::default();
    vars.set("token", "t-secret");
    let step = runner()
        .run_request(&request, &mut vars)
        .await
        .expect("the request reached the mock");

    let received = api.requests_to("/post");
    assert_eq!(received.len(), 1);
    let sent = &received[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.header("x-tenant"), Some("acme"));
    assert_eq!(sent.header("authorization"), Some("Bearer t-secret"));
    assert_eq!(sent.body, "{\"hello\": \"mock\", \"tenant\": \"acme\"}");

    assert!(step.passed, "{:?}", step.failures());
    assert_eq!(step.script_tests.len(), 1);
    assert_eq!(step.script_tests[0].name, "the mock answered");
    assert_eq!(vars.get("echoed"), Some("POST"));
    assert_eq!(
        step.var_sets.get("echoed").map(String::as_str),
        Some("POST")
    );

    let second = collection::load_request(ws, &slug, "api.http#1").unwrap();
    runner()
        .run_request(&second, &mut vars)
        .await
        .expect("the second request reached the mock");
    let read_back = api.requests_to("/get");
    assert_eq!(read_back.len(), 1);
    assert_eq!(
        read_back[0].query.get("page").map(String::as_str),
        Some("2")
    );
}

/// A file-scoped `@var` is more local than the environment, so it wins.
#[tokio::test]
async fn a_file_variable_beats_the_environment_of_the_same_name() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;
    mandalo_testkit::fixtures::environment(ws, "local", &[("who", "from-environment")]);

    write(
        ws,
        &slug,
        "api.http",
        &format!(
            "@who = from-file\n\n### Ping\nGET {}\nX-Who: {{{{who}}}}\n",
            api.url("/get")
        ),
    );

    let request = collection::load_request(ws, &slug, "api.http#0").unwrap();
    runner()
        .run_one(ws, &request, Some("local"))
        .await
        .expect("the request reached the mock");

    assert_eq!(
        api.requests_to("/get")[0].header("x-who"),
        Some("from-file")
    );
}

/// The whole file is a suite, in the order the file writes it.
#[tokio::test]
async fn a_suite_run_walks_every_block_in_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;

    write(
        ws,
        &slug,
        "chain.http",
        &format!(
            "\
### Login
POST {}

{{\"user\": \"ada\"}}

> {{%
pm.environment.set(\"token\", \"t-from-login\");
%}}

### Use the token
GET {}
Authorization: Bearer {{{{token}}}}
",
            api.url("/post"),
            api.url("/get")
        ),
    );

    let report = runner().run_suite(ws, &slug, None, None).await.unwrap();
    assert_eq!(report.total, 2, "{:?}", report.steps);
    assert!(report.passed, "{:?}", report.steps);
    assert_eq!(
        report
            .steps
            .iter()
            .map(|s| s.path.as_str())
            .collect::<Vec<_>>(),
        vec!["chain.http#0", "chain.http#1"]
    );
    assert_eq!(
        api.requests_to("/get")[0].header("authorization"),
        Some("Bearer t-from-login")
    );
}

/// A `< file` body reads from the workspace, and only from inside it.
#[tokio::test]
async fn a_file_body_is_read_from_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;
    std::fs::create_dir_all(ws.join("fixtures")).unwrap();
    std::fs::write(ws.join("fixtures/payload.json"), "{\"from\":\"a file\"}").unwrap();

    write(
        ws,
        &slug,
        "upload.http",
        &format!("POST {}\n\n< ./fixtures/payload.json\n", api.url("/post")),
    );

    let request = collection::load_request(ws, &slug, "upload.http#0").unwrap();
    let step = runner()
        .run_one(ws, &request, None)
        .await
        .expect("the request reached the mock");
    assert!(step.error.is_none(), "{:?}", step.error);
    assert_eq!(api.requests_to("/post")[0].body, "{\"from\":\"a file\"}");
}

#[tokio::test]
async fn a_graphql_block_posts_the_document_and_its_variables() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;

    write(
        ws,
        &slug,
        "gql.http",
        &format!(
            "\
### User
POST {}
X-REQUEST-TYPE: GraphQL

query User($id: ID!) {{ user(id: $id) {{ name }} }}

{{\"id\": \"u-1\"}}
",
            api.url("/graphql")
        ),
    );

    let summary = &collection::list_tree(ws).unwrap().collections[0].requests[0];
    assert_eq!(summary.kind, "graphql");

    let request = collection::load_request(ws, &slug, "gql.http#0").unwrap();
    runner()
        .run_one(ws, &request, None)
        .await
        .expect("the request reached the mock");

    let sent = &api.requests_to("/graphql")[0];
    assert!(
        sent.headers
            .iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("x-request-type")),
        "the marker header must never reach the wire: {:?}",
        sent.headers
    );
    let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(
        body["query"],
        "query User($id: ID!) { user(id: $id) { name } }"
    );
    assert_eq!(body["variables"], serde_json::json!({"id": "u-1"}));
}

/// A `proto:` path is workspace-relative in the file and reaches the compiler as an
/// absolute path proven to be inside the workspace.
#[tokio::test]
async fn a_grpc_file_calls_the_mock_with_a_workspace_relative_proto() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;
    std::fs::create_dir_all(ws.join("protos")).unwrap();
    std::fs::copy(api.proto_path(), ws.join("protos/mock.proto")).unwrap();

    write(
        ws,
        &slug,
        "mock.grpc",
        &format!(
            "\
### Say hello
{}/mock.v1.Mock/Say
proto: protos/mock.proto
x-trace: mandalo

{{\"text\": \"hola\", \"count\": 21}}

> {{%
pm.test(\"doubled\", () => {{}});
%}}
",
            api.grpc_url()
        ),
    );

    let summary = &collection::list_tree(ws).unwrap().collections[0].requests[0];
    assert_eq!(summary.kind, "grpc");
    assert_eq!(summary.path, "mock.grpc#0");
    assert_eq!(summary.name, "Say hello");

    let request = collection::load_request(ws, &slug, "mock.grpc#0").unwrap();
    let resolved = request.grpc.as_ref().expect("a gRPC call");
    assert!(
        Path::new(&resolved.proto_paths[0]).is_absolute(),
        "{:?}",
        resolved.proto_paths
    );

    let step = runner()
        .run_one(ws, &request, None)
        .await
        .expect("the call reached the mock");
    assert!(step.error.is_none(), "{:?}", step.error);
    let body: serde_json::Value =
        serde_json::from_str(&step.grpc.expect("a gRPC response").body).unwrap();
    assert_eq!(body["doubled"], 42);
    assert_eq!(step.script_tests.len(), 1);
}

#[test]
fn a_proto_path_that_escapes_the_workspace_never_reaches_the_compiler() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    write(
        ws,
        &slug,
        "mock.grpc",
        "grpc://x:1/a.B/C\nproto: ../../etc/passwd\n",
    );

    let tree = collection::list_tree(ws).unwrap();
    assert!(tree.collections[0].requests.is_empty());
    assert!(tree.skipped[0].contains("workspace"), "{:?}", tree.skipped);
    assert_eq!(
        collection::load_request(ws, &slug, "mock.grpc#0")
            .unwrap_err()
            .code(),
        "E_PATH_ESCAPE"
    );
}

/// Editing one request must leave every other byte of the file alone.
#[test]
fn saving_a_request_back_rewrites_only_what_changed() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let source = "\
### Login
# the token is short-lived
POST https://x.dev/login
Content-Type: application/json

{\"user\": \"ada\"}

### Profile
GET https://x.dev/me
";
    write(ws, &slug, "api.http", source);

    let unchanged = collection::load_request_source(ws, &slug, "api.http#1").unwrap();
    collection::save_request(ws, &slug, Some("api.http#1"), None, &unchanged).unwrap();
    let on_disk =
        std::fs::read_to_string(ws.join("collections").join(&slug).join("api.http")).unwrap();
    assert_eq!(
        on_disk, source,
        "a save that changed nothing rewrote the file"
    );

    let mut edited = unchanged.clone();
    edited.url = "https://x.dev/profile".to_string();
    let saved = collection::save_request(ws, &slug, Some("api.http#1"), None, &edited).unwrap();
    assert_eq!(saved.path, "api.http#1");
    let on_disk =
        std::fs::read_to_string(ws.join("collections").join(&slug).join("api.http")).unwrap();
    assert_eq!(
        on_disk,
        source.replace("https://x.dev/me", "https://x.dev/profile")
    );
    assert!(on_disk.contains("# the token is short-lived"));
}

#[test]
fn a_request_toml_left_in_a_collection_is_reported_not_silently_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    write(ws, &slug, "legacy.toml", "id = \"a\"\nname = \"Legacy\"\nkind = \"http\"\nmethod = \"GET\"\nurl = \"https://x.dev\"\n");

    let tree = collection::list_tree(ws).unwrap();
    assert!(tree.collections[0].requests.is_empty());
    assert_eq!(tree.skipped.len(), 1);
    assert!(
        tree.skipped[0].contains("legacy.toml"),
        "{:?}",
        tree.skipped
    );
    assert!(tree.skipped[0].contains("converted"), "{:?}", tree.skipped);

    assert_eq!(
        collection::load_request(ws, &slug, "legacy.toml")
            .unwrap_err()
            .code(),
        "E_UNSUPPORTED"
    );
}

/// The two form-data spellings are one format: the readable one Mándalo writes and
/// the literal one an import brings must reach the server as the same parts.
#[tokio::test]
async fn both_form_data_spellings_put_the_same_parts_on_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;
    std::fs::create_dir_all(ws.join("files")).unwrap();
    for (name, body) in [("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")] {
        std::fs::write(ws.join("files").join(name), body).unwrap();
    }

    let url = api.url("/body/multipart");
    write(
        ws,
        &slug,
        "readable.http",
        &format!(
            "@dir = files

### Readable
POST {url}
Content-Type: multipart/form-data

caption = two attachments, one field
attachments = < ./files/a.txt < ./files/b.txt < ./{{{{dir}}}}/c.txt
avatar = < ./files/a.txt; type=image/png
"
        ),
    );
    write(
        ws,
        &slug,
        "literal.http",
        &format!(
            "### Literal
POST {url}
Content-Type: multipart/form-data; boundary=WebAppBoundary

--WebAppBoundary
Content-Disposition: form-data; name=\"caption\"

two attachments, one field
--WebAppBoundary
Content-Disposition: form-data; name=\"attachments\"; filename=\"a.txt\"

< files/a.txt
--WebAppBoundary
Content-Disposition: form-data; name=\"attachments\"; filename=\"b.txt\"

< files/b.txt
--WebAppBoundary
Content-Disposition: form-data; name=\"attachments\"; filename=\"c.txt\"

< files/c.txt
--WebAppBoundary
Content-Disposition: form-data; name=\"avatar\"; filename=\"a.txt\"
Content-Type: image/png

< files/a.txt
--WebAppBoundary--
"
        ),
    );

    let readable = collection::load_request(ws, &slug, "readable.http#0").unwrap();
    let literal = collection::load_request(ws, &slug, "literal.http#0").unwrap();
    assert_eq!(readable.body, literal.body, "one model, two spellings");

    let mut echoes = Vec::new();
    for request in [&readable, &literal] {
        let step = runner()
            .run_one(ws, request, None)
            .await
            .expect("the request reached the mock");
        let body = step.response.expect("a response").body;
        echoes.push(serde_json::from_str::<serde_json::Value>(&body).expect("a JSON echo"));
    }
    // Every part, in order, byte for byte. Only the boundary differs, and it is the
    // HTTP client that mints it, per request.
    assert_eq!(echoes[0]["parts"], echoes[1]["parts"]);
    assert_eq!(echoes[0]["count"], echoes[1]["count"]);

    let echo = &echoes[0];
    assert_eq!(echo["count"], 5);
    let names: Vec<&str> = echo["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "caption",
            "attachments",
            "attachments",
            "attachments",
            "avatar"
        ],
        "three files on one field arrive as three parts sharing the name, in order"
    );
    assert_eq!(echo["parts"][0]["text"], "two attachments, one field");
    assert_eq!(echo["parts"][1]["filename"], "a.txt");
    assert_eq!(echo["parts"][2]["text"], "beta");
    assert_eq!(echo["parts"][3]["text"], "gamma");
    assert_eq!(echo["parts"][4]["contentType"], "image/png");
}

#[tokio::test]
async fn a_form_data_path_outside_the_workspace_is_refused_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let slug = workspace(ws);
    let api = MockApi::start().await;

    write(
        ws,
        &slug,
        "leak.http",
        &format!(
            "### Leak
POST {}
Content-Type: multipart/form-data

secret = < ../../../etc/passwd
",
            api.url("/body/multipart")
        ),
    );

    let error = collection::load_request(ws, &slug, "leak.http#0").unwrap_err();
    assert_eq!(error.code(), "E_PATH_ESCAPE");
    assert!(api.received_requests().is_empty());
}
