use mandalo_core::assertions::{JsonOp, StatusOp, TestAssertion};
use mandalo_core::collection::SavedRequest;
use mandalo_core::runner::{StepResult, VarFrame};
use mandalo_core::Body;
use mandalo_testkit::fixtures::{request, runner};
use mandalo_testkit::MockApi;

fn graphql(api: &MockApi, query: &str, variables: &str) -> SavedRequest {
    let mut req = request("GraphQL", "GET", &api.url("/graphql"));
    req.kind = "graphql".to_string();
    req.body = Body::graphql(query, variables);
    req
}

async fn send(req: &SavedRequest, vars: &mut VarFrame) -> StepResult {
    runner()
        .run_request(req, vars)
        .await
        .expect("the request reached the mock")
}

#[tokio::test]
async fn a_query_with_variables_is_sent_as_the_graphql_envelope_over_post() {
    let api = MockApi::start().await;
    let req = graphql(
        &api,
        "query User($id: ID!) { user(id: $id) { id name email } }",
        "{\"id\": \"u-7\"}",
    );

    let step = send(&req, &mut VarFrame::default()).await;

    let received = api.last_request().expect("a recorded request");
    assert_eq!(received.method, "POST");
    assert_eq!(received.header("content-type"), Some("application/json"));
    let sent = received.json_body();
    assert_eq!(
        sent["query"],
        "query User($id: ID!) { user(id: $id) { id name email } }"
    );
    assert_eq!(sent["variables"], serde_json::json!({ "id": "u-7" }));

    let body: serde_json::Value =
        serde_json::from_str(&step.response.expect("a response").body).expect("json response");
    assert_eq!(body["data"]["user"]["id"], "u-7");
    assert_eq!(body["data"]["user"]["name"], "Ada Lovelace");
}

#[tokio::test]
async fn variables_interpolate_from_the_variable_frame() {
    let api = MockApi::start().await;
    let req = graphql(
        &api,
        "query User($id: ID!) { user(id: $id) { id } }",
        "{\"id\": \"{{userId}}\"}",
    );
    let mut vars = VarFrame::default();
    vars.set("userId", "u-42");

    send(&req, &mut vars).await;

    let sent = api.last_request().expect("a recorded request").json_body();
    assert_eq!(sent["variables"], serde_json::json!({ "id": "u-42" }));
}

#[tokio::test]
async fn a_mutation_reaches_the_resolver_with_its_variables() {
    let api = MockApi::start().await;
    let req = graphql(
        &api,
        "mutation Create($name: String!) { createUser(name: $name) { id name } }",
        "{\"name\": \"nova\"}",
    );

    let step = send(&req, &mut VarFrame::default()).await;

    let body: serde_json::Value =
        serde_json::from_str(&step.response.expect("a response").body).expect("json response");
    assert_eq!(body["data"]["createUser"]["name"], "nova");
}

/// A GraphQL `errors` payload arrives with HTTP 200. The client does not treat that as a
/// failure — a status-only assertion passes. Only a json assertion on `$.errors` catches it.
#[tokio::test]
async fn graphql_errors_arrive_with_http_200_and_only_a_json_assertion_catches_them() {
    let api = MockApi::start().await;
    let mut req = graphql(&api, "{ boom }", "");
    req.tests = vec![TestAssertion::Status {
        op: StatusOp::Eq,
        value: 200,
    }];

    let step = send(&req, &mut VarFrame::default()).await;

    assert!(
        step.passed,
        "a status-only assertion still passes on a GraphQL error payload"
    );
    let response = step.response.expect("a response");
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_str(&response.body).expect("json response");
    assert_eq!(body["errors"][0]["extensions"]["code"], "BOOM");

    req.tests = vec![TestAssertion::Json {
        path: "$.errors".to_string(),
        op: JsonOp::Absent,
        value: None,
    }];
    let guarded = send(&req, &mut VarFrame::default()).await;
    assert!(!guarded.passed);
    assert_eq!(guarded.failures().len(), 1);
}

#[tokio::test]
async fn a_malformed_json_response_fails_the_assertion_that_reads_it() {
    let api = MockApi::start().await;
    let mut req = graphql(&api, "{ malformed }", "");
    req.tests = vec![TestAssertion::Json {
        path: "$.data".to_string(),
        op: JsonOp::Exists,
        value: None,
    }];

    let step = send(&req, &mut VarFrame::default()).await;

    assert!(!step.passed);
    assert!(
        step.failures()[0].contains("not JSON"),
        "{:?}",
        step.failures()
    );
    assert_eq!(
        step.response.expect("a response").body,
        "{\"data\": {\"malformed\":"
    );
}

#[tokio::test]
async fn a_graphql_request_is_forced_to_post_whatever_the_saved_method_says() {
    let api = MockApi::start().await;
    let mut req = graphql(&api, "{ users { id name } }", "");
    req.method = "DELETE".to_string();

    send(&req, &mut VarFrame::default()).await;

    assert_eq!(
        api.last_request().expect("a recorded request").method,
        "POST"
    );
}

#[tokio::test]
async fn a_user_content_type_is_not_duplicated_by_the_graphql_default() {
    let api = MockApi::start().await;
    let mut req = graphql(&api, "{ users { id } }", "");
    req.headers = vec![(
        "content-type".to_string(),
        "application/graphql-response+json".to_string(),
    )];

    send(&req, &mut VarFrame::default()).await;

    assert_eq!(
        api.last_request()
            .expect("a recorded request")
            .headers_named("content-type"),
        vec!["application/graphql-response+json"]
    );
}
