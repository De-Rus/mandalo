use crate::http::ReceivedRequest;
use axum::extract::Extension;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

fn json_response(status: StatusCode, body: String) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn user(id: &str) -> Value {
    json!({
        "id": id,
        "name": "Ada Lovelace",
        "email": "ada@example.com"
    })
}

pub(crate) async fn handle(Extension(received): Extension<ReceivedRequest>) -> Response {
    let envelope = received.json_body();
    let query = envelope["query"].as_str().unwrap_or_default();
    let variables = &envelope["variables"];

    if envelope.is_null() || query.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "errors": [{ "message": "a graphql request needs a query field" }] })
                .to_string(),
        );
    }

    if query.contains("malformed") {
        return json_response(StatusCode::OK, "{\"data\": {\"malformed\":".to_string());
    }

    if query.contains("boom") {
        return json_response(
            StatusCode::OK,
            json!({
                "data": null,
                "errors": [{
                    "message": "boom: the resolver refused",
                    "path": ["boom"],
                    "extensions": { "code": "BOOM" }
                }]
            })
            .to_string(),
        );
    }

    if query.contains("createUser") {
        let name = variables["name"].as_str().unwrap_or("unnamed");
        return json_response(
            StatusCode::OK,
            json!({ "data": { "createUser": { "id": "u-2", "name": name } } }).to_string(),
        );
    }

    if query.contains("users") {
        return json_response(
            StatusCode::OK,
            json!({ "data": { "users": [user("u-1"), user("u-2")] } }).to_string(),
        );
    }

    if query.contains("user") {
        let id = variables["id"]
            .as_str()
            .map(String::from)
            .or_else(|| variables["id"].as_i64().map(|n| n.to_string()))
            .unwrap_or_else(|| "u-1".to_string());
        return json_response(
            StatusCode::OK,
            json!({ "data": { "user": user(&id) } }).to_string(),
        );
    }

    json_response(
        StatusCode::OK,
        json!({ "data": null, "errors": [{ "message": format!("unknown query: {query}") }] })
            .to_string(),
    )
}
