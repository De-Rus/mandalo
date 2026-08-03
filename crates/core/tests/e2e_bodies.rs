use mandalo_core::request::{self, RequestSpec, ResponseData};
use mandalo_core::{Body, FormDataRow, FormRow};
use mandalo_testkit::MockApi;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff];

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("files")).unwrap();
    std::fs::write(dir.path().join("files/avatar.png"), PNG).unwrap();
    std::fs::write(dir.path().join("files/a.txt"), b"alpha").unwrap();
    std::fs::write(dir.path().join("files/b.txt"), b"beta").unwrap();
    std::fs::write(dir.path().join("files/c.txt"), b"gamma").unwrap();
    std::fs::write(dir.path().join("files/report.pdf"), b"%PDF-1.7 report").unwrap();
    dir
}

fn spec(ws: &Path, url: &str, body: Body) -> RequestSpec {
    RequestSpec {
        kind: "http".to_string(),
        method: "POST".to_string(),
        url: url.to_string(),
        headers: Vec::new(),
        body,
        auth: Default::default(),
        vars: BTreeMap::new(),
        workspace: Some(ws.to_path_buf()),
    }
}

async fn send(spec: RequestSpec) -> ResponseData {
    request::send_request(spec)
        .await
        .expect("the request reached the mock")
}

fn json(response: &ResponseData) -> Value {
    serde_json::from_str(&response.body).expect("a JSON echo")
}

fn part(echo: &Value, index: usize) -> &Value {
    &echo["parts"][index]
}

#[tokio::test]
async fn form_data_sends_text_and_file_parts_the_server_can_read() {
    let ws = workspace();
    let api = MockApi::start().await;
    let response = send(spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![
                FormDataRow::text("caption", "hola"),
                FormDataRow::file("avatar", "files/avatar.png"),
                FormDataRow {
                    enabled: false,
                    ..FormDataRow::text("debug", "1")
                },
                FormDataRow {
                    enabled: false,
                    ..FormDataRow::file("secret", "files/report.pdf")
                },
            ],
        },
    ))
    .await;
    let echo = json(&response);

    assert_eq!(echo["count"], 2);
    assert!(echo["contentType"]
        .as_str()
        .unwrap()
        .starts_with("multipart/form-data; boundary="));
    assert_eq!(part(&echo, 0)["name"], "caption");
    assert_eq!(part(&echo, 0)["text"], "hola");
    assert_eq!(part(&echo, 0)["filename"], Value::Null);
    assert_eq!(part(&echo, 1)["name"], "avatar");
    assert_eq!(part(&echo, 1)["filename"], "avatar.png");
    assert_eq!(part(&echo, 1)["contentType"], "image/png");
    assert_eq!(part(&echo, 1)["size"], PNG.len());
    assert_eq!(part(&echo, 1)["bytes"], serde_json::json!(PNG));
}

#[tokio::test]
async fn one_form_data_field_carrying_three_files_arrives_as_three_parts_in_order() {
    let ws = workspace();
    let api = MockApi::start().await;
    let response = send(spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![
                FormDataRow::files("attachments", ["files/a.txt", "files/b.txt", "files/c.txt"]),
                FormDataRow::text("note", "three files"),
            ],
        },
    ))
    .await;
    let echo = json(&response);

    assert_eq!(echo["count"], 4);
    let names: Vec<&str> = echo["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["attachments", "attachments", "attachments", "note"]
    );
    for (index, (file, text)) in [("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")]
        .into_iter()
        .enumerate()
    {
        assert_eq!(part(&echo, index)["filename"], file);
        assert_eq!(part(&echo, index)["text"], text);
        assert_eq!(part(&echo, index)["contentType"], "text/plain");
    }
}

#[tokio::test]
async fn a_row_content_type_covers_every_file_in_that_row() {
    let ws = workspace();
    let api = MockApi::start().await;
    let response = send(spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![FormDataRow {
                content_type: Some("application/x-fixture".to_string()),
                ..FormDataRow::files("docs", ["files/a.txt", "files/report.pdf"])
            }],
        },
    ))
    .await;
    let echo = json(&response);

    assert_eq!(echo["count"], 2);
    assert_eq!(part(&echo, 0)["contentType"], "application/x-fixture");
    assert_eq!(part(&echo, 1)["contentType"], "application/x-fixture");
}

#[tokio::test]
async fn the_same_field_name_on_separate_rows_repeats_the_name() {
    let ws = workspace();
    let api = MockApi::start().await;
    let response = send(spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![
                FormDataRow::file("attachments", "files/a.txt"),
                FormDataRow::text("attachments", "inline"),
                FormDataRow::file("attachments", "files/b.txt"),
            ],
        },
    ))
    .await;
    let echo = json(&response);

    assert_eq!(echo["count"], 3);
    assert_eq!(part(&echo, 0)["filename"], "a.txt");
    assert_eq!(part(&echo, 1)["text"], "inline");
    assert_eq!(part(&echo, 1)["filename"], Value::Null);
    assert_eq!(part(&echo, 2)["filename"], "b.txt");
    for index in 0..3 {
        assert_eq!(part(&echo, index)["name"], "attachments");
    }
}

#[tokio::test]
async fn form_data_interpolates_keys_values_and_file_paths() {
    let ws = workspace();
    let api = MockApi::start().await;
    let mut spec = spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![
                FormDataRow::text("{{field}}", "hola {{who}}"),
                FormDataRow::file("avatar", "files/{{picture}}.png"),
            ],
        },
    );
    spec.vars = BTreeMap::from([
        ("field".to_string(), "caption".to_string()),
        ("who".to_string(), "nova".to_string()),
        ("picture".to_string(), "avatar".to_string()),
    ]);
    let echo = json(&send(spec).await);

    assert_eq!(part(&echo, 0)["name"], "caption");
    assert_eq!(part(&echo, 0)["text"], "hola nova");
    assert_eq!(part(&echo, 1)["filename"], "avatar.png");
}

#[tokio::test]
async fn a_user_content_type_never_clobbers_the_multipart_boundary() {
    let ws = workspace();
    let api = MockApi::start().await;
    let mut spec = spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![FormDataRow::text("a", "1")],
        },
    );
    spec.headers = vec![(
        "Content-Type".to_string(),
        "multipart/form-data".to_string(),
    )];
    let echo = json(&send(spec).await);

    assert_eq!(echo["count"], 1);
    assert!(echo["contentType"].as_str().unwrap().contains("boundary="));
    let received = api.last_request().unwrap();
    assert_eq!(received.headers_named("content-type").len(), 1);
}

#[tokio::test]
async fn urlencoded_arrives_percent_encoded_without_disabled_rows() {
    let ws = workspace();
    let api = MockApi::start().await;
    let mut spec = spec(
        ws.path(),
        &api.url("/body/urlencoded"),
        Body::Urlencoded {
            rows: vec![
                FormRow::new("user", "{{user}}"),
                FormRow::new("q", "a b&c=d"),
                FormRow {
                    enabled: false,
                    ..FormRow::new("debug", "1")
                },
            ],
        },
    );
    spec.vars = BTreeMap::from([("user".to_string(), "ada lovelace".to_string())]);
    let echo = json(&send(spec).await);

    assert_eq!(echo["contentType"], "application/x-www-form-urlencoded");
    assert_eq!(echo["raw"], "user=ada+lovelace&q=a+b%26c%3Dd");
    assert_eq!(
        echo["pairs"],
        serde_json::json!([["user", "ada lovelace"], ["q", "a b&c=d"]])
    );
}

#[tokio::test]
async fn a_binary_body_sends_the_bytes_with_a_sniffed_content_type() {
    let ws = workspace();
    let api = MockApi::start().await;
    let echo = json(
        &send(spec(
            ws.path(),
            &api.url("/body/binary"),
            Body::Binary {
                file: "files/avatar.png".to_string(),
                content_type: None,
            },
        ))
        .await,
    );

    assert_eq!(echo["contentType"], "image/png");
    assert_eq!(echo["size"], PNG.len());
    assert_eq!(echo["bytes"], serde_json::json!(PNG));
}

#[tokio::test]
async fn a_user_content_type_wins_over_the_sniffed_binary_one() {
    let ws = workspace();
    let api = MockApi::start().await;
    let mut spec = spec(
        ws.path(),
        &api.url("/body/binary"),
        Body::Binary {
            file: "files/report.pdf".to_string(),
            content_type: None,
        },
    );
    spec.headers = vec![(
        "content-type".to_string(),
        "application/x-report".to_string(),
    )];
    let echo = json(&send(spec).await);

    assert_eq!(echo["contentType"], "application/x-report");
    assert_eq!(echo["text"], "%PDF-1.7 report");
    assert_eq!(
        api.last_request()
            .unwrap()
            .headers_named("content-type")
            .len(),
        1
    );
}

#[tokio::test]
async fn a_raw_body_carries_the_content_type_of_its_language() {
    let ws = workspace();
    let api = MockApi::start().await;
    let response = send(spec(
        ws.path(),
        &api.url("/post"),
        Body::json("{\"name\": \"nova\"}"),
    ))
    .await;

    let received = api.last_request().unwrap();
    assert_eq!(received.header("content-type"), Some("application/json"));
    assert_eq!(json(&response)["body"], "{\"name\": \"nova\"}");
}

#[tokio::test]
async fn a_file_outside_the_workspace_stops_the_send() {
    let ws = workspace();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("id_rsa"), b"PRIVATE KEY").unwrap();
    let api = MockApi::start().await;

    for file in [
        "../../../etc/passwd".to_string(),
        outside.path().join("id_rsa").to_string_lossy().into_owned(),
    ] {
        let error = request::send_request(spec(
            ws.path(),
            &api.url("/body/binary"),
            Body::Binary {
                file: file.clone(),
                content_type: None,
            },
        ))
        .await
        .unwrap_err();
        assert_eq!(error.code(), "E_PATH_ESCAPE", "{file}");
    }
    assert!(api.received_requests().is_empty());
}

#[tokio::test]
async fn one_bad_file_in_a_multi_file_row_names_the_field_and_sends_nothing() {
    let ws = workspace();
    let api = MockApi::start().await;
    let error = request::send_request(spec(
        ws.path(),
        &api.url("/body/multipart"),
        Body::Formdata {
            rows: vec![FormDataRow::files(
                "attachments",
                ["files/a.txt", "files/ghost.txt"],
            )],
        },
    ))
    .await
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("form-data field \"attachments\""),
        "{error}"
    );
    assert!(error.to_string().contains("ghost.txt"), "{error}");
    assert!(api.received_requests().is_empty());
}

#[tokio::test]
async fn a_body_file_without_a_workspace_root_fails_loud() {
    let api = MockApi::start().await;
    let mut spec = spec(
        Path::new("/nowhere"),
        &api.url("/body/binary"),
        Body::Binary {
            file: "files/avatar.png".to_string(),
            content_type: None,
        },
    );
    spec.workspace = None;
    let error = request::send_request(spec).await.unwrap_err();

    assert!(error.to_string().contains("no workspace root"), "{error}");
}

#[tokio::test]
async fn the_example_workspace_upload_requests_run_green_end_to_end() {
    let api = MockApi::start().await;
    let dir = tempfile::tempdir().unwrap();
    mandalo_testkit::fixtures::install_example_workspace(
        dir.path(),
        &api.base_url(),
        &api.grpc_url(),
        &api.proto_path(),
    );

    let paths: Vec<String> = mandalo_core::collection::list_tree(dir.path())
        .unwrap()
        .collections
        .into_iter()
        .find(|c| c.slug == "mock")
        .expect("the mock collection")
        .requests
        .into_iter()
        .map(|r| r.path)
        .filter(|p| p.starts_with("upload.http#"))
        .collect();
    assert_eq!(paths.len(), 3, "{paths:?}");

    let runner = mandalo_core::Runner::new(mandalo_core::NoSecrets, mandalo_core::AllowAll);
    for path in paths {
        let request = mandalo_core::collection::load_request(dir.path(), "mock", &path).unwrap();
        let step = runner
            .run_one(dir.path(), &request, Some("local"))
            .await
            .expect("the request runs");
        assert!(step.passed, "{path}: {:#?}", step.failures());
    }
}
