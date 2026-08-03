use mandalo_core::grpc::{compile, DynamicCodec};
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, Value};
use std::net::SocketAddr;
use std::path::Path;
use tokio::sync::oneshot;
use tonic::codegen::http;
use tonic::codegen::{Body, BoxFuture, Context, Poll, Service, StdError};
use tonic::server::NamedService;

pub const PROTO: &str = r#"syntax = "proto3";

package mock.v1;

enum Tier {
  TIER_UNSPECIFIED = 0;
  TIER_FREE = 1;
  TIER_PRO = 2;
}

message Address {
  string city = 1;
  string country = 2;
}

message User {
  string id = 1;
  string name = 2;
  Address address = 3;
  repeated string tags = 4;
  Tier tier = 5;
}

message EchoRequest {
  string text = 1;
  int32 count = 2;
}

message EchoResponse {
  string text = 1;
  int32 doubled = 2;
  string trace = 3;
}

message GetUserRequest {
  string id = 1;
  repeated string tags = 2;
  Tier tier = 3;
}

message GetUserResponse {
  User user = 1;
}

message FailRequest {
  string reason = 1;
}

message FailResponse {
  string never = 1;
}

message SlowRequest {
  int32 ms = 1;
}

message SlowResponse {
  string done = 1;
}

message TickRequest {
  int32 count = 1;
}

message Tick {
  int32 n = 1;
}

service Mock {
  rpc Say(EchoRequest) returns (EchoResponse);
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc Fail(FailRequest) returns (FailResponse);
  rpc Slow(SlowRequest) returns (SlowResponse);
  rpc Ticks(TickRequest) returns (stream Tick);
}
"#;

const MAX_SLOW_MS: i32 = 10_000;

#[derive(Clone, Copy)]
enum Op {
    Say,
    GetUser,
    Fail,
    Slow,
}

impl Op {
    fn from_path(path: &str) -> Option<Op> {
        match path {
            "/mock.v1.Mock/Say" => Some(Op::Say),
            "/mock.v1.Mock/GetUser" => Some(Op::GetUser),
            "/mock.v1.Mock/Fail" => Some(Op::Fail),
            "/mock.v1.Mock/Slow" => Some(Op::Slow),
            _ => None,
        }
    }

    fn input(self) -> &'static str {
        match self {
            Op::Say => "mock.v1.EchoRequest",
            Op::GetUser => "mock.v1.GetUserRequest",
            Op::Fail => "mock.v1.FailRequest",
            Op::Slow => "mock.v1.SlowRequest",
        }
    }

    fn output(self) -> &'static str {
        match self {
            Op::Say => "mock.v1.EchoResponse",
            Op::GetUser => "mock.v1.GetUserResponse",
            Op::Fail => "mock.v1.FailResponse",
            Op::Slow => "mock.v1.SlowResponse",
        }
    }
}

#[derive(Clone)]
struct MockGrpc {
    pool: DescriptorPool,
    log: bool,
}

impl NamedService for MockGrpc {
    const NAME: &'static str = "mock.v1.Mock";
}

impl<B> Service<http::Request<B>> for MockGrpc
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

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        let path = request.uri().path().to_string();
        if self.log {
            println!("{:<7} {path:<28} grpc", "RPC");
        }
        let Some(op) = Op::from_path(&path) else {
            return Box::pin(async move {
                Ok(http::Response::builder()
                    .status(200)
                    .header("grpc-status", "12")
                    .header("grpc-message", "method not implemented by the mock")
                    .header("content-type", "application/grpc")
                    .body(tonic::codegen::empty_body())
                    .expect("unimplemented response"))
            });
        };
        let codec = DynamicCodec::new(
            self.pool
                .get_message_by_name(op.input())
                .expect("input descriptor"),
        );
        let handler = MockHandler {
            op,
            pool: self.pool.clone(),
        };
        Box::pin(async move {
            let mut grpc = tonic::server::Grpc::new(codec);
            Ok(grpc.unary(handler, request).await)
        })
    }
}

struct MockHandler {
    op: Op,
    pool: DescriptorPool,
}

impl MockHandler {
    fn message(&self, name: &str) -> DynamicMessage {
        DynamicMessage::new(
            self.pool
                .get_message_by_name(name)
                .expect("message descriptor"),
        )
    }
}

fn string_field(msg: &DynamicMessage, name: &str) -> String {
    msg.get_field_by_name(name)
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn i32_field(msg: &DynamicMessage, name: &str) -> i32 {
    msg.get_field_by_name(name)
        .and_then(|v| v.as_i32())
        .unwrap_or_default()
}

impl tonic::server::UnaryService<DynamicMessage> for MockHandler {
    type Response = DynamicMessage;
    type Future = BoxFuture<tonic::Response<DynamicMessage>, tonic::Status>;

    fn call(&mut self, request: tonic::Request<DynamicMessage>) -> Self::Future {
        let op = self.op;
        let mut out = self.message(op.output());
        let user_descriptor: MessageDescriptor = self
            .pool
            .get_message_by_name("mock.v1.User")
            .expect("user descriptor");
        let address_descriptor: MessageDescriptor = self
            .pool
            .get_message_by_name("mock.v1.Address")
            .expect("address descriptor");

        Box::pin(async move {
            let trace = request
                .metadata()
                .get("x-trace")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("missing")
                .to_string();
            let input = request.into_inner();

            match op {
                Op::Say => {
                    let text = string_field(&input, "text");
                    let count = i32_field(&input, "count");
                    out.set_field_by_name("text", Value::String(text));
                    out.set_field_by_name("doubled", Value::I32(count.saturating_mul(2)));
                    out.set_field_by_name("trace", Value::String(trace));
                }
                Op::GetUser => {
                    let tags = input
                        .get_field_by_name("tags")
                        .and_then(|v| v.as_list().map(<[Value]>::to_vec))
                        .unwrap_or_default();
                    let tier = input
                        .get_field_by_name("tier")
                        .and_then(|v| v.as_enum_number())
                        .unwrap_or_default();
                    let mut address = DynamicMessage::new(address_descriptor);
                    address.set_field_by_name("city", Value::String("Madrid".to_string()));
                    address.set_field_by_name("country", Value::String("ES".to_string()));
                    let mut user = DynamicMessage::new(user_descriptor);
                    user.set_field_by_name("id", Value::String(string_field(&input, "id")));
                    user.set_field_by_name("name", Value::String("Ada Lovelace".to_string()));
                    user.set_field_by_name("address", Value::Message(address));
                    user.set_field_by_name("tags", Value::List(tags));
                    user.set_field_by_name("tier", Value::EnumNumber(tier));
                    out.set_field_by_name("user", Value::Message(user));
                }
                Op::Fail => {
                    let reason = string_field(&input, "reason");
                    return Err(tonic::Status::with_details(
                        tonic::Code::FailedPrecondition,
                        format!("the mock refused: {reason}"),
                        bytes::Bytes::from_static(b"mock-details"),
                    ));
                }
                Op::Slow => {
                    let ms = i32_field(&input, "ms").clamp(0, MAX_SLOW_MS);
                    tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
                    out.set_field_by_name("done", Value::String(format!("slept {ms}ms")));
                }
            }
            Ok(tonic::Response::new(out))
        })
    }
}

pub(crate) async fn spawn(
    bind: std::net::IpAddr,
    port: u16,
    proto_path: &Path,
    log: bool,
) -> (SocketAddr, oneshot::Sender<()>) {
    let pool = compile(&[proto_path.to_string_lossy().into_owned()]).expect("compile mock.proto");
    let listener = tokio::net::TcpListener::bind((bind, port))
        .await
        .expect("bind mock grpc port");
    let addr = listener.local_addr().expect("mock grpc address");
    let (stop, stopped) = oneshot::channel();
    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .accept_http1(true)
            .layer(crate::cors::grpc_web_cors())
            .layer(tonic_web::GrpcWebLayer::new())
            .add_service(MockGrpc { pool, log })
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async move {
                    let _ = stopped.await;
                },
            )
            .await;
    });
    (addr, stop)
}
