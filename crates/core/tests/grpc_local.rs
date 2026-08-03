use mandalo_core::grpc::{send_grpc, DynamicCodec, GrpcSpec};
use prost_reflect::{DynamicMessage, MessageDescriptor, Value};
use tonic::codegen::http;
use tonic::codegen::{Body, BoxFuture, Context, Poll, Service, StdError};
use tonic::server::NamedService;

const PROTO: &str = r#"
syntax = "proto3";
package test.v1;

message EchoRequest {
  string text = 1;
  int32 count = 2;
}

message EchoResponse {
  string text = 1;
  int32 doubled = 2;
}

service Echo {
  rpc Say(EchoRequest) returns (EchoResponse);
}
"#;

#[derive(Clone)]
struct EchoService {
    input: MessageDescriptor,
    output: MessageDescriptor,
}

impl NamedService for EchoService {
    const NAME: &'static str = "test.v1.Echo";
}

impl<B> Service<http::Request<B>> for EchoService
where
    B: Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::BoxBody>;
    type Error = std::convert::Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        match req.uri().path() {
            "/test.v1.Echo/Say" => {
                let codec = DynamicCodec::new(self.input.clone());
                let handler = SayHandler {
                    output: self.output.clone(),
                };
                Box::pin(async move {
                    let mut grpc = tonic::server::Grpc::new(codec);
                    Ok(grpc.unary(handler, req).await)
                })
            }
            _ => Box::pin(async move {
                Ok(http::Response::builder()
                    .status(200)
                    .header("grpc-status", "12")
                    .header("content-type", "application/grpc")
                    .body(tonic::codegen::empty_body())
                    .unwrap())
            }),
        }
    }
}

struct SayHandler {
    output: MessageDescriptor,
}

impl tonic::server::UnaryService<DynamicMessage> for SayHandler {
    type Response = DynamicMessage;
    type Future = BoxFuture<tonic::Response<DynamicMessage>, tonic::Status>;

    fn call(&mut self, request: tonic::Request<DynamicMessage>) -> Self::Future {
        let output = self.output.clone();
        Box::pin(async move {
            let trace = request
                .metadata()
                .get("x-trace")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("missing")
                .to_string();
            let msg = request.into_inner();
            let text = msg
                .get_field_by_name("text")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let count = msg
                .get_field_by_name("count")
                .and_then(|v| v.as_i32())
                .unwrap_or_default();
            if text == "boom" {
                return Err(tonic::Status::invalid_argument("boom is not allowed"));
            }
            let mut out = DynamicMessage::new(output);
            out.set_field_by_name("text", Value::String(format!("{text}|trace={trace}")));
            out.set_field_by_name("doubled", Value::I32(count * 2));
            Ok(tonic::Response::new(out))
        })
    }
}

fn write_proto() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("echo.proto");
    std::fs::write(&path, PROTO).unwrap();
    let path = path.to_str().unwrap().to_string();
    (dir, path)
}

async fn spawn_echo_server(proto_path: &str) -> String {
    let pool = mandalo_core::grpc::compile(&[proto_path.to_string()]).unwrap();
    let svc = EchoService {
        input: pool.get_message_by_name("test.v1.EchoRequest").unwrap(),
        output: pool.get_message_by_name("test.v1.EchoResponse").unwrap(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(svc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn spec(json: serde_json::Value) -> GrpcSpec {
    serde_json::from_value(json).unwrap()
}

#[tokio::test]
async fn send_grpc_round_trips_message_and_metadata_via_local_server() {
    let (_dir, proto_path) = write_proto();
    let url = spawn_echo_server(&proto_path).await;

    let resp = send_grpc(spec(serde_json::json!({
        "url": url,
        "protoPaths": [proto_path],
        "service": "test.v1.Echo",
        "method": "Say",
        "message": "{\"text\": \"{{saludo}}\", \"count\": 21}",
        "metadata": [["x-trace", "{{trace}}"]],
        "vars": {"saludo": "hola", "trace": "t-42"}
    })))
    .await
    .unwrap();

    let body: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
    assert_eq!(body["text"], "hola|trace=t-42");
    assert_eq!(body["doubled"], 42);
}

#[tokio::test]
async fn send_grpc_surfaces_server_status_errors() {
    let (_dir, proto_path) = write_proto();
    let url = spawn_echo_server(&proto_path).await;

    let err = send_grpc(spec(serde_json::json!({
        "url": url,
        "protoPaths": [proto_path],
        "service": "test.v1.Echo",
        "method": "Say",
        "message": "{\"text\": \"boom\"}"
    })))
    .await
    .unwrap_err();

    assert_eq!(err.code(), "E_GRPC");
    let err = err.to_string();
    assert!(err.contains("InvalidArgument"));
    assert!(err.contains("boom is not allowed"));
}
