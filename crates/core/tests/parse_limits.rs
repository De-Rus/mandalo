use mandalo_core::body::{self, MAX_BODY_FILE_BYTES};
use mandalo_core::grpc_format::GrpcDoc;
use mandalo_core::http_format::HttpDoc;
use mandalo_core::text_format::{MAX_VARS_BYTES, MAX_VAR_BYTES};
use std::collections::BTreeMap;

/// The shape that turns 588 bytes of `.http` into gigabytes: every definition is
/// four copies of the one above it, and each is stored already expanded.
fn doubling_vars(depth: usize) -> String {
    let mut out = String::from("@a0 = xxxx\n");
    for step in 1..depth {
        out.push_str(&format!(
            "@a{step} = {0}{0}{0}{0}\n",
            format_args!("{{{{a{}}}}}", step - 1)
        ));
    }
    out.push_str("\n### Go\nGET https://x.dev/{{a");
    out.push_str(&(depth - 1).to_string());
    out.push_str("}}\n");
    out
}

#[test]
fn exponential_var_expansion_fails_instead_of_allocating() {
    let source = doubling_vars(15);
    assert!(source.len() < 1024, "the fixture is tiny: {}", source.len());
    let error = HttpDoc::parse("api", &source)
        .expect("the file parses")
        .resolved(0)
        .unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(
        error.to_string().contains(&MAX_VAR_BYTES.to_string())
            || error.to_string().contains(&MAX_VARS_BYTES.to_string()),
        "{error}"
    );
}

#[test]
fn exponential_var_expansion_fails_in_grpc_files_too() {
    let mut source = doubling_vars(15);
    source.truncate(source.find("\n### Go").expect("the request marker"));
    source
        .push_str("\n\n### Go\ngrpc://localhost:50051/mock.v1.Mock/Echo\n\n{\"t\": \"{{a14}}\"}\n");
    let error = GrpcDoc::parse("mock", &source)
        .expect("the file parses")
        .resolved(0)
        .unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
}

#[test]
fn nested_vars_within_the_limit_still_resolve() {
    let source = "\
@host = x.dev
@base = https://{{host}}/v1

### Go
GET {{base}}/me
";
    let block = HttpDoc::parse("api", source)
        .expect("the file parses")
        .resolved(0)
        .expect("the variables resolve");
    assert_eq!(block.request.url, "https://x.dev/v1/me");
}

#[test]
fn a_body_file_past_the_limit_is_refused_before_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.bin");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_BODY_FILE_BYTES + 1).unwrap();
    drop(file);

    let error = body::read_file(Some(dir.path()), "huge.bin", None, &BTreeMap::new())
        .err()
        .expect("a file past the limit is refused");
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(
        error.to_string().contains(&MAX_BODY_FILE_BYTES.to_string())
            && error.to_string().contains("huge.bin"),
        "{error}"
    );
}

#[test]
fn a_small_body_file_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("payload.json"), b"{\"ok\": true}").unwrap();
    let resolved =
        body::read_file(Some(dir.path()), "payload.json", None, &BTreeMap::new()).unwrap();
    assert_eq!(resolved.bytes, b"{\"ok\": true}");
    assert_eq!(resolved.content_type, "application/json");
}
