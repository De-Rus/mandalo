use mandalo_core::assertions::Scripts;
use mandalo_core::body::Body;
use mandalo_core::collection::{GrpcRequest, SavedRequest};
use mandalo_core::grpc_format::{render_request, GrpcDoc};
use mandalo_core::request::Auth;

const MULTI: &str = "\
### Say hello
{{grpcUrl}}/mock.v1.Mock/Say
proto: protos/mock.proto
x-trace: mandalo

{\"text\": \"hola\", \"count\": 21}

### Get user
{{grpcUrl}}/mock.v1.Mock/GetUser
proto: protos/mock.proto

{\"id\": \"u-1\"}
";

fn doc(source: &str) -> GrpcDoc {
    GrpcDoc::parse("mock", source).expect("the file parses")
}

fn request_at(source: &str, index: usize) -> SavedRequest {
    doc(source).raw(index).expect("the request exists").request
}

#[test]
fn a_file_holds_many_calls_separated_by_hashes() {
    let parsed = doc(MULTI);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed.names(), vec!["Say hello", "Get user"]);

    let say = parsed.raw(0).unwrap().request;
    assert_eq!(say.kind, "grpc");
    assert_eq!(say.url, "{{grpcUrl}}");
    let grpc = say.grpc.expect("a gRPC call");
    assert_eq!(grpc.service, "mock.v1.Mock");
    assert_eq!(grpc.method, "Say");
    assert_eq!(grpc.proto_paths, vec!["protos/mock.proto"]);
    assert_eq!(
        grpc.metadata,
        vec![("x-trace".to_string(), "mandalo".to_string())]
    );
    assert_eq!(grpc.message, "{\"text\": \"hola\", \"count\": 21}");
}

#[test]
fn the_call_line_is_the_real_grpc_path_so_a_full_url_works_too() {
    let request = request_at("http://localhost:50051/mock.v1.Mock/Say\n\n{}\n", 0);
    assert_eq!(request.url, "http://localhost:50051");
    let grpc = request.grpc.unwrap();
    assert_eq!(grpc.service, "mock.v1.Mock");
    assert_eq!(grpc.method, "Say");
}

#[test]
fn a_call_line_without_a_target_fails_loud_with_its_line() {
    let error = GrpcDoc::parse("mock", "### A\nmock.v1.Mock/Say\n").unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(error.to_string().contains("line 2"), "{error}");
    assert!(error.to_string().contains("names no target"), "{error}");
}

#[test]
fn proto_paths_are_workspace_relative_and_reject_traversal() {
    for bad in ["../mock.proto", "/tmp/mock.proto", "a/../../b.proto"] {
        let error =
            GrpcDoc::parse("mock", &format!("{{{{u}}}}/a.B/C\nproto: {bad}\n")).unwrap_err();
        assert_eq!(error.code(), "E_PATH_ESCAPE", "{bad}: {error}");
    }
}

#[test]
fn several_proto_lines_accumulate_in_order() {
    let request = request_at("{{u}}/a.B/C\nproto: a.proto\nproto: b.proto\n", 0);
    assert_eq!(
        request.grpc.unwrap().proto_paths,
        vec!["a.proto", "b.proto"]
    );
}

#[test]
fn a_reserved_looking_key_fails_loud_rather_than_being_sent_as_metadata() {
    for key in ["url", "service", "method", "message", "protos", "import"] {
        let error = GrpcDoc::parse("mock", &format!("{{{{u}}}}/a.B/C\n{key}: x\n")).unwrap_err();
        assert_eq!(error.code(), "E_UNSUPPORTED", "{key}: {error}");
        assert!(
            error.to_string().contains("reserved key is `proto:`"),
            "{error}"
        );
    }
}

#[test]
fn the_protocols_own_metadata_prefix_is_refused() {
    let error = GrpcDoc::parse("mock", "{{u}}/a.B/C\ngrpc-timeout: 1S\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("`grpc-`"), "{error}");
}

#[test]
fn vars_reach_the_target_the_message_the_metadata_and_the_proto_path() {
    let source = "@dir = protos\n@host = grpc://localhost:1\n\n{{host}}/a.B/C\nproto: {{dir}}/mock.proto\nx-t: {{dir}}\n\n{\"d\": \"{{dir}}\"}\n";
    let parsed = doc(source);
    assert_eq!(parsed.raw(0).unwrap().request.url, "{{host}}");

    let resolved = parsed.resolved(0).unwrap().request;
    assert_eq!(resolved.url, "grpc://localhost:1");
    let grpc = resolved.grpc.unwrap();
    assert_eq!(grpc.proto_paths, vec!["protos/mock.proto"]);
    assert_eq!(
        grpc.metadata,
        vec![("x-t".to_string(), "protos".to_string())]
    );
    assert_eq!(grpc.message, "{\"d\": \"protos\"}");
}

#[test]
fn script_blocks_map_onto_the_same_pm_engine() {
    let source = "\
### Say
< {%
pm.environment.set(\"n\", \"1\");
%}
{{u}}/a.B/C
proto: a.proto

{}

> {%
pm.test(\"ok\", () => {});
%}
";
    let request = request_at(source, 0);
    assert_eq!(
        request.scripts,
        Scripts {
            pre: Some("pm.environment.set(\"n\", \"1\");".to_string()),
            post: Some("pm.test(\"ok\", () => {});".to_string()),
        }
    );
}

#[test]
fn crlf_and_a_missing_trailing_newline_parse() {
    let crlf = doc(&MULTI.replace('\n', "\r\n"));
    assert_eq!(crlf.len(), 2);
    let bare = doc("{{u}}/a.B/C\nproto: a.proto");
    assert_eq!(bare.len(), 1);
}

#[test]
fn parsing_and_reserialising_an_untouched_file_is_byte_identical() {
    let crlf = MULTI.replace('\n', "\r\n");
    let gnarly = "\
@host = grpc://localhost:1

# a note

###   Spaced
{{host}}/a.B/C
proto:   a.proto
x-t:  1


{\"a\": 1}

> {%
pm.test(\"x\", () => {});
%}
";
    for source in [MULTI, crlf.as_str(), "{{u}}/a.B/C\nproto: a.proto", gnarly] {
        let parsed = doc(source);
        assert_eq!(parsed.source(), source);
        for index in 0..parsed.len() {
            let request = parsed.raw(index).unwrap().request;
            assert_eq!(
                parsed.replace(index, &request).unwrap(),
                source,
                "re-saving request {index} unchanged rewrote the file"
            );
        }
    }
}

#[test]
fn a_metadata_edit_touches_one_line() {
    let parsed = doc(MULTI);
    let mut request = parsed.raw(0).unwrap().request;
    request.grpc.as_mut().unwrap().metadata = vec![("x-trace".to_string(), "t-2".to_string())];
    assert_eq!(
        parsed.replace(0, &request).unwrap(),
        MULTI.replace("x-trace: mandalo", "x-trace: t-2")
    );
}

#[test]
fn rendering_a_request_and_parsing_it_back_returns_the_same_request() {
    let shape = SavedRequest {
        id: "mock-0".to_string(),
        name: "Say hello".to_string(),
        kind: "grpc".to_string(),
        method: "POST".to_string(),
        url: "{{grpcUrl}}".to_string(),
        description: None,
        headers: Vec::new(),
        auth: Auth::None,
        body: Body::None,
        grpc: Some(GrpcRequest {
            proto_paths: vec!["protos/mock.proto".to_string()],
            service: "mock.v1.Mock".to_string(),
            method: "Say".to_string(),
            message: "{\"text\": \"hola\"}".to_string(),
            metadata: vec![("x-trace".to_string(), "mandalo".to_string())],
        }),
        scripts: Scripts {
            pre: None,
            post: Some("pm.test(\"ok\", () => {});".to_string()),
        },
        tests: Vec::new(),
        captures: Vec::new(),
    };
    let text = render_request(&shape, "\n").expect("renders");
    let back = GrpcDoc::parse("mock", &text)
        .unwrap_or_else(|e| panic!("does not parse back: {e}\n{text}"))
        .raw(0)
        .unwrap()
        .request;
    assert_eq!(back, shape, "{text}");
}

#[test]
fn an_http_request_cannot_be_written_into_a_grpc_file() {
    let mut shape = request_at(MULTI, 0);
    shape.kind = "http".to_string();
    let error = render_request(&shape, "\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
}

#[test]
fn a_request_is_addressed_by_index_and_by_name() {
    let parsed = doc(MULTI);
    assert_eq!(parsed.index_of("1").unwrap(), 1);
    assert_eq!(parsed.index_of("Say hello").unwrap(), 0);
    assert_eq!(parsed.index_of("9").unwrap_err().code(), "E_NOT_FOUND");
}

#[test]
fn a_request_can_be_appended_and_removed() {
    let parsed = doc(MULTI);
    let mut fresh = parsed.raw(0).unwrap().request;
    fresh.name = "Third".to_string();
    let grown = doc(&parsed.append(&fresh).unwrap());
    assert_eq!(grown.len(), 3);
    assert_eq!(grown.remove(2).unwrap().trim_end(), MULTI.trim_end());
}

#[test]
fn a_description_becomes_a_comment_on_a_new_block_and_cannot_be_edited_as_a_field() {
    let mut fresh = doc(MULTI).raw(0).unwrap().request;
    fresh.description = Some("doubles the count".to_string());
    let text = render_request(&fresh, "\n").expect("a new block carries it as a comment");
    assert!(text.contains("# doubles the count\n"), "{text}");
    assert_eq!(
        GrpcDoc::parse("mock", &text)
            .unwrap()
            .raw(0)
            .unwrap()
            .request
            .description,
        None
    );
    assert_eq!(
        doc(MULTI).replace(0, &fresh).unwrap_err().code(),
        "E_UNSUPPORTED"
    );
}
