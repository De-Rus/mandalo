use mandalo_core::collection::{GrpcRequest, SavedRequest};
use mandalo_core::grpc::list_grpc_methods;
use mandalo_core::runner::VarFrame;
use mandalo_testkit::fixtures::{request, runner};
use mandalo_testkit::MockApi;

fn grpc(api: &MockApi, method: &str, message: &str) -> SavedRequest {
    let mut req = request(method, "POST", &api.grpc_url());
    req.kind = "grpc".to_string();
    req.grpc = Some(GrpcRequest {
        proto_paths: vec![api.proto_path()],
        service: "mock.v1.Mock".to_string(),
        method: method.to_string(),
        message: message.to_string(),
        metadata: Vec::new(),
    });
    req
}

fn body(step: &mandalo_core::runner::StepResult) -> serde_json::Value {
    serde_json::from_str(&step.grpc.as_ref().expect("a grpc response").body).expect("json response")
}

#[tokio::test]
async fn a_unary_call_round_trips_through_the_runner() {
    let api = MockApi::start().await;
    let req = grpc(&api, "Say", "{\"text\": \"hola\", \"count\": 21}");

    let step = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect("the call reached the mock");

    assert_eq!(body(&step)["text"], "hola");
    assert_eq!(body(&step)["doubled"], 42);
    assert!(step.passed);
}

#[tokio::test]
async fn nested_messages_repeated_fields_and_enums_survive_the_round_trip() {
    let api = MockApi::start().await;
    let req = grpc(
        &api,
        "GetUser",
        "{\"id\": \"u-1\", \"tags\": [\"a\", \"b\"], \"tier\": \"TIER_PRO\"}",
    );

    let step = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect("the call reached the mock");

    let user = &body(&step)["user"];
    assert_eq!(user["id"], "u-1");
    assert_eq!(user["address"]["city"], "Madrid");
    assert_eq!(user["tags"], serde_json::json!(["a", "b"]));
    assert_eq!(user["tier"], "TIER_PRO");
}

#[tokio::test]
async fn metadata_is_sent_interpolated_and_echoed_back() {
    let api = MockApi::start().await;
    let mut req = grpc(&api, "Say", "{\"text\": \"{{saludo}}\"}");
    req.grpc.as_mut().expect("grpc body").metadata =
        vec![("x-trace".to_string(), "{{trace}}".to_string())];
    let mut vars = VarFrame::default();
    vars.set("saludo", "hola");
    vars.set("trace", "t-42");

    let step = runner()
        .run_request(&req, &mut vars)
        .await
        .expect("the call reached the mock");

    assert_eq!(body(&step)["text"], "hola");
    assert_eq!(body(&step)["trace"], "t-42");
}

#[tokio::test]
async fn a_server_status_error_propagates_with_its_message() {
    let api = MockApi::start().await;
    let req = grpc(&api, "Fail", "{\"reason\": \"no credit\"}");

    let error = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect_err("the mock refuses this call");

    assert_eq!(error.code(), "E_GRPC");
    assert!(error.to_string().contains("FailedPrecondition"), "{error}");
    assert!(
        error.to_string().contains("the mock refused: no credit"),
        "{error}"
    );
}

#[tokio::test]
async fn a_server_streaming_method_fails_loud_before_any_connection() {
    let api = MockApi::start().await;
    let req = grpc(&api, "Ticks", "{\"count\": 3}");

    let error = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect_err("streaming is not supported");

    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert_eq!(error.to_string(), "streaming methods not supported yet");
}

#[tokio::test]
async fn list_grpc_methods_reports_every_method_with_its_streaming_flags() {
    let api = MockApi::start().await;

    let methods = list_grpc_methods(vec![api.proto_path()])
        .await
        .expect("the proto compiles");

    let names: Vec<&str> = methods.iter().map(|m| m.method.as_str()).collect();
    assert_eq!(names, vec!["Say", "GetUser", "Fail", "Slow", "Ticks"]);
    let say = &methods[0];
    assert_eq!(say.service, "mock.v1.Mock");
    assert_eq!(say.input, "mock.v1.EchoRequest");
    assert_eq!(say.output, "mock.v1.EchoResponse");
    assert!(!say.client_streaming);
    assert!(!say.server_streaming);
    let ticks = methods.last().expect("Ticks");
    assert!(ticks.server_streaming);
    assert!(!ticks.client_streaming);
}

#[tokio::test]
async fn a_slow_unary_call_still_completes() {
    let api = MockApi::start().await;
    let req = grpc(&api, "Slow", "{\"ms\": 200}");

    let step = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect("the call reached the mock");

    assert_eq!(body(&step)["done"], "slept 200ms");
    let grpc_response = step.grpc.expect("a grpc response");
    assert!(grpc_response.duration_ms >= 200);
}

#[tokio::test]
async fn a_dead_port_is_a_network_error() {
    let api = MockApi::start().await;
    let mut req = grpc(&api, "Say", "{\"text\": \"hola\"}");
    req.url = "http://127.0.0.1:1".to_string();

    let error = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect_err("nothing listens on port 1");

    assert_eq!(error.code(), "E_NETWORK");
    assert!(error.to_string().contains("failed to connect"), "{error}");
}

#[tokio::test]
async fn an_unknown_method_on_a_reachable_server_fails_loud() {
    let api = MockApi::start().await;
    let req = grpc(&api, "Nope", "{}");

    let error = runner()
        .run_request(&req, &mut VarFrame::default())
        .await
        .expect_err("the proto has no such method");

    assert_eq!(error.code(), "E_NOT_FOUND");
    assert!(error.to_string().contains("method not found"), "{error}");
}

/// BUG: `send_grpc` interpolates the url, the message and the metadata, but never
/// `protoPaths`, so a saved gRPC request cannot point at a per-environment proto file —
/// the raw `{{...}}` reaches `protox` and fails with "proto path has no parent directory".
/// Ignored until proto paths are interpolated like every other field.
#[tokio::test]
#[ignore = "protoPaths is not interpolated"]
async fn a_proto_path_can_come_from_a_variable() {
    let api = MockApi::start().await;
    let mut req = grpc(&api, "Say", "{\"text\": \"hola\"}");
    req.grpc.as_mut().expect("grpc body").proto_paths = vec!["{{protoPath}}".to_string()];
    let mut vars = VarFrame::default();
    vars.set("protoPath", api.proto_path());

    let step = runner()
        .run_request(&req, &mut vars)
        .await
        .expect("the call reached the mock");

    assert_eq!(body(&step)["text"], "hola");
}
