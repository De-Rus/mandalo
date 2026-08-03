use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GrpcMethodInfo {
    pub service: String,
    pub method: String,
    pub input: String,
    pub output: String,
    pub client_streaming: bool,
    pub server_streaming: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrpcSpec {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub proto_paths: Vec<String>,
    pub service: String,
    pub method: String,
    pub message: String,
    #[serde(default)]
    pub metadata: Vec<(String, String)>,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrpcResponse {
    pub body: String,
    pub duration_ms: u128,
}

pub fn compile(proto_paths: &[String]) -> CoreResult<DescriptorPool> {
    if proto_paths.is_empty() {
        return Err(CoreError::Request("no proto files given".to_string()));
    }
    let mut includes: Vec<PathBuf> = Vec::new();
    for p in proto_paths {
        let parent = Path::new(p)
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .ok_or_else(|| CoreError::Request(format!("proto path has no parent directory: {p}")))?
            .to_path_buf();
        if !includes.contains(&parent) {
            includes.push(parent);
        }
    }
    let set =
        protox::compile(proto_paths, &includes).map_err(|e| CoreError::Parse(e.to_string()))?;
    DescriptorPool::from_file_descriptor_set(set).map_err(|e| CoreError::Parse(e.to_string()))
}

fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_end + 3;
    let authority_end = url[authority_start..]
        .find('/')
        .map(|i| authority_start + i)
        .unwrap_or(url.len());
    match url[authority_start..authority_end].rfind('@') {
        Some(at) => format!(
            "{}<redacted>{}",
            &url[..authority_start],
            &url[authority_start + at..]
        ),
        None => url.to_string(),
    }
}

fn find_method(pool: &DescriptorPool, service: &str, method: &str) -> CoreResult<MethodDescriptor> {
    let svc = pool
        .get_service_by_name(service)
        .ok_or_else(|| CoreError::NotFound(format!("service not found: {service}")))?;
    let found = svc.methods().find(|m| m.name() == method);
    found.ok_or_else(|| CoreError::NotFound(format!("method not found: {service}/{method}")))
}

fn ensure_unary(method: &MethodDescriptor) -> CoreResult<()> {
    if method.is_client_streaming() || method.is_server_streaming() {
        return Err(CoreError::Unsupported(
            "streaming methods not supported yet".to_string(),
        ));
    }
    Ok(())
}

fn parse_message(descriptor: MessageDescriptor, json: &str) -> CoreResult<DynamicMessage> {
    let mut de = serde_json::Deserializer::from_str(json);
    let msg = DynamicMessage::deserialize(descriptor, &mut de)
        .map_err(|e| CoreError::Parse(format!("invalid message JSON: {e}")))?;
    de.end()
        .map_err(|e| CoreError::Parse(format!("invalid message JSON: {e}")))?;
    Ok(msg)
}

#[derive(Clone)]
pub struct DynamicCodec {
    decoded: MessageDescriptor,
}

impl DynamicCodec {
    pub fn new(decoded: MessageDescriptor) -> Self {
        Self { decoded }
    }
}

pub struct DynamicEncoder;

pub struct DynamicDecoder {
    decoded: MessageDescriptor,
}

impl tonic::codec::Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            decoded: self.decoded.clone(),
        }
    }
}

impl tonic::codec::Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = tonic::Status;

    fn encode(
        &mut self,
        item: DynamicMessage,
        dst: &mut tonic::codec::EncodeBuf<'_>,
    ) -> Result<(), Self::Error> {
        prost::Message::encode(&item, dst).map_err(|e| tonic::Status::internal(e.to_string()))
    }
}

impl tonic::codec::Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = tonic::Status;

    fn decode(
        &mut self,
        src: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Self::Item>, Self::Error> {
        DynamicMessage::decode(self.decoded.clone(), src)
            .map(Some)
            .map_err(|e| tonic::Status::internal(e.to_string()))
    }
}

pub async fn list_grpc_methods(proto_paths: Vec<String>) -> CoreResult<Vec<GrpcMethodInfo>> {
    tokio::task::spawn_blocking(move || grpc_methods(&proto_paths))
        .await
        .map_err(|e| CoreError::Grpc(format!("proto compilation task failed: {e}")))?
}

pub fn grpc_methods(proto_paths: &[String]) -> CoreResult<Vec<GrpcMethodInfo>> {
    let pool = compile(proto_paths)?;
    let mut out = Vec::new();
    for service in pool.services() {
        for method in service.methods() {
            out.push(GrpcMethodInfo {
                service: service.full_name().to_string(),
                method: method.name().to_string(),
                input: method.input().full_name().to_string(),
                output: method.output().full_name().to_string(),
                client_streaming: method.is_client_streaming(),
                server_streaming: method.is_server_streaming(),
            });
        }
    }
    Ok(out)
}

#[derive(Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ProtoFieldType {
    String,
    Number,
    Bool,
    Bytes,
    Enum,
    Message,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProtoField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: ProtoFieldType,
    pub repeated: bool,
    pub message: Option<MessageShape>,
    pub enum_values: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MessageShape {
    pub name: String,
    pub fields: Vec<ProtoField>,
}

const MAX_DEPTH: usize = 8;
const MAX_NODES: usize = 2_000;
const LISTED_TYPES: usize = 10;

/// Well-known types whose canonical JSON is not an object, so describing them by
/// their `seconds`/`nanos` fields would hand the user an example the encoder
/// rejects. They are described as the JSON value they actually take.
fn well_known_scalar(name: &str) -> Option<ProtoFieldType> {
    match name {
        "google.protobuf.Timestamp"
        | "google.protobuf.Duration"
        | "google.protobuf.FieldMask"
        | "google.protobuf.StringValue"
        | "google.protobuf.Int64Value"
        | "google.protobuf.UInt64Value" => Some(ProtoFieldType::String),
        "google.protobuf.BytesValue" => Some(ProtoFieldType::Bytes),
        "google.protobuf.BoolValue" => Some(ProtoFieldType::Bool),
        "google.protobuf.Int32Value"
        | "google.protobuf.UInt32Value"
        | "google.protobuf.FloatValue"
        | "google.protobuf.DoubleValue" => Some(ProtoFieldType::Number),
        _ => None,
    }
}

/// `{}` and `[]` are the whole of these types' JSON, and their fields describe an
/// internal representation the encoder never accepts.
fn well_known_opaque(name: &str) -> Option<bool> {
    match name {
        "google.protobuf.Struct" | "google.protobuf.Value" | "google.protobuf.Empty" => Some(false),
        "google.protobuf.ListValue" => Some(true),
        _ => None,
    }
}

fn kind_label(kind: &Kind) -> String {
    match kind {
        Kind::Double => "double",
        Kind::Float => "float",
        Kind::Int32 => "int32",
        Kind::Int64 => "int64",
        Kind::Uint32 => "uint32",
        Kind::Uint64 => "uint64",
        Kind::Sint32 => "sint32",
        Kind::Sint64 => "sint64",
        Kind::Fixed32 => "fixed32",
        Kind::Fixed64 => "fixed64",
        Kind::Sfixed32 => "sfixed32",
        Kind::Sfixed64 => "sfixed64",
        Kind::Bool => "bool",
        Kind::String => "string",
        Kind::Bytes => "bytes",
        Kind::Message(m) => m.full_name(),
        Kind::Enum(e) => e.full_name(),
    }
    .to_string()
}

/// A `map<k, v>` is a repeated synthetic entry message on the wire but a plain
/// JSON object in a written message, so it is described as one message with no
/// fields: the example comes out `{}`, which encodes, and the shape's name still
/// carries the key and value types for anything that wants to render them.
fn map_shape(field: &FieldDescriptor) -> MessageShape {
    let entry = match field.kind() {
        Kind::Message(m) => m,
        _ => {
            return MessageShape {
                name: "map".to_string(),
                fields: Vec::new(),
            }
        }
    };
    let label = |number: u32| {
        entry
            .get_field(number)
            .map(|f| kind_label(&f.kind()))
            .unwrap_or_else(|| "?".to_string())
    };
    MessageShape {
        name: format!("map<{}, {}>", label(1), label(2)),
        fields: Vec::new(),
    }
}

struct Walk {
    path: Vec<String>,
    budget: usize,
}

impl Walk {
    fn message(&mut self, descriptor: &MessageDescriptor, depth: usize) -> MessageShape {
        self.path.push(descriptor.full_name().to_string());
        let fields = descriptor
            .fields()
            .map(|f| self.field(&f, depth))
            .collect::<Vec<_>>();
        self.path.pop();
        MessageShape {
            name: descriptor.full_name().to_string(),
            fields,
        }
    }

    /// A nested message is expanded unless it is already on the path (a type that
    /// contains itself), the tree is deeper than `MAX_DEPTH`, or the node budget
    /// is spent. In all three cases the field stays, and `message: null` says the
    /// example stops here — a truncated skeleton beats an error or a hang.
    fn nested(&mut self, descriptor: &MessageDescriptor, depth: usize) -> Option<MessageShape> {
        let name = descriptor.full_name();
        if depth >= MAX_DEPTH || self.budget == 0 || self.path.iter().any(|seen| seen == name) {
            return None;
        }
        self.budget -= 1;
        Some(self.message(descriptor, depth + 1))
    }

    fn field(&mut self, field: &FieldDescriptor, depth: usize) -> ProtoField {
        let name = field.name().to_string();
        let plain = |field_type, repeated| ProtoField {
            name: name.clone(),
            field_type,
            repeated,
            message: None,
            enum_values: Vec::new(),
        };

        if field.is_map() {
            return ProtoField {
                message: Some(map_shape(field)),
                ..plain(ProtoFieldType::Message, false)
            };
        }
        let repeated = field.is_list();

        match field.kind() {
            Kind::String => plain(ProtoFieldType::String, repeated),
            Kind::Bool => plain(ProtoFieldType::Bool, repeated),
            Kind::Bytes => plain(ProtoFieldType::Bytes, repeated),
            Kind::Enum(values) => ProtoField {
                enum_values: values.values().map(|v| v.name().to_string()).collect(),
                ..plain(ProtoFieldType::Enum, repeated)
            },
            Kind::Message(nested) => {
                if let Some(field_type) = well_known_scalar(nested.full_name()) {
                    return plain(field_type, repeated);
                }
                if let Some(as_list) = well_known_opaque(nested.full_name()) {
                    return plain(ProtoFieldType::Message, repeated || as_list);
                }
                ProtoField {
                    message: self.nested(&nested, depth),
                    ..plain(ProtoFieldType::Message, repeated)
                }
            }
            _ => plain(ProtoFieldType::Number, repeated),
        }
    }
}

fn unknown_type(pool: &DescriptorPool, type_name: &str) -> CoreError {
    let known: Vec<String> = pool
        .all_messages()
        .filter(|m| !m.is_map_entry() && !m.full_name().starts_with("google.protobuf."))
        .map(|m| m.full_name().to_string())
        .collect();
    let listed = known
        .iter()
        .take(LISTED_TYPES)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let rest = known.len().saturating_sub(LISTED_TYPES);
    let defines = match (known.is_empty(), rest) {
        (true, _) => "define no messages at all".to_string(),
        (false, 0) => format!("define: {listed}"),
        (false, more) => format!("define: {listed} …and {more} more"),
    };
    CoreError::NotFound(format!(
        "message type not found: {type_name}. These protos {defines}"
    ))
}

pub fn message_shape(proto_paths: &[String], type_name: &str) -> CoreResult<MessageShape> {
    let pool = compile(proto_paths)?;
    shape_in(&pool, type_name)
}

pub fn shape_in(pool: &DescriptorPool, type_name: &str) -> CoreResult<MessageShape> {
    let descriptor = pool
        .get_message_by_name(type_name)
        .ok_or_else(|| unknown_type(pool, type_name))?;
    let mut walk = Walk {
        path: Vec::new(),
        budget: MAX_NODES,
    };
    Ok(walk.message(&descriptor, 0))
}

pub async fn describe_message(
    proto_paths: Vec<String>,
    type_name: String,
) -> CoreResult<MessageShape> {
    tokio::task::spawn_blocking(move || message_shape(&proto_paths, &type_name))
        .await
        .map_err(|e| CoreError::Grpc(format!("proto compilation task failed: {e}")))?
}

pub async fn send_grpc(spec: GrpcSpec) -> CoreResult<GrpcResponse> {
    let vars = &spec.vars;
    let url = interpolate::apply(&spec.url, vars)?;
    let message = interpolate::apply(&spec.message, vars)?;
    let mut metadata = Vec::with_capacity(spec.metadata.len());
    for (k, v) in &spec.metadata {
        metadata.push((k.clone(), interpolate::apply(v, vars)?));
    }

    let mut proto_paths = Vec::with_capacity(spec.proto_paths.len());
    for p in &spec.proto_paths {
        proto_paths.push(interpolate::apply(p, vars)?);
    }

    let pool = compile(&proto_paths)?;
    let method = find_method(&pool, &spec.service, &spec.method)?;
    ensure_unary(&method)?;
    let request_message = parse_message(method.input(), &message)?;

    let mut request = tonic::Request::new(request_message);
    for (k, v) in metadata {
        let key: tonic::metadata::MetadataKey<tonic::metadata::Ascii> = k
            .parse()
            .map_err(|_| CoreError::Request(format!("invalid metadata key (ascii only): {k}")))?;
        let value: tonic::metadata::AsciiMetadataValue = v.parse().map_err(|_| {
            CoreError::Request(format!("invalid metadata value for key {k} (ascii only)"))
        })?;
        request.metadata_mut().insert(key, value);
    }

    let endpoint = if url.starts_with("http://") {
        tonic::transport::Endpoint::from_shared(url.clone())
            .map_err(|e| CoreError::Request(e.to_string()))?
    } else if url.starts_with("https://") {
        tonic::transport::Endpoint::from_shared(url.clone())
            .map_err(|e| CoreError::Request(e.to_string()))?
            .tls_config(tonic::transport::ClientTlsConfig::new().with_native_roots())
            .map_err(|e| CoreError::Network(e.to_string()))?
    } else {
        return Err(CoreError::Request(format!(
            "grpc url must start with http:// or https://: {}",
            redact_url(&url)
        )));
    };
    let endpoint = endpoint
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CALL_TIMEOUT);

    let channel = endpoint.connect().await.map_err(|e| {
        CoreError::Network(format!("failed to connect to {}: {e}", redact_url(&url)))
    })?;
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| CoreError::Network(e.to_string()))?;

    let path: http::uri::PathAndQuery = format!("/{}/{}", spec.service, spec.method)
        .parse()
        .map_err(|e: http::uri::InvalidUri| CoreError::Request(e.to_string()))?;

    let codec = DynamicCodec::new(method.output());
    let started = Instant::now();
    let response = grpc
        .unary(request, path, codec)
        .await
        .map_err(|s| CoreError::Grpc(format!("grpc error {:?}: {}", s.code(), s.message())))?;
    let body = serde_json::to_string_pretty(response.get_ref())
        .map_err(|e| CoreError::Parse(e.to_string()))?;
    Ok(GrpcResponse {
        body,
        duration_ms: started.elapsed().as_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTO: &str = r#"
syntax = "proto3";
package test.v1;

message EchoRequest {
  string text = 1;
  int32 count = 2;
}

message EchoResponse {
  string text = 1;
}

service Echo {
  rpc Say(EchoRequest) returns (EchoResponse);
  rpc StreamOut(EchoRequest) returns (stream EchoResponse);
  rpc StreamIn(stream EchoRequest) returns (EchoResponse);
}
"#;

    fn write_proto() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("echo.proto");
        std::fs::write(&path, PROTO).unwrap();
        let path = path.to_str().unwrap().to_string();
        (dir, path)
    }

    #[test]
    fn lists_all_service_methods_with_streaming_flags() {
        let (_dir, path) = write_proto();
        let methods = grpc_methods(&[path]).unwrap();
        assert_eq!(
            methods,
            vec![
                GrpcMethodInfo {
                    service: "test.v1.Echo".into(),
                    method: "Say".into(),
                    input: "test.v1.EchoRequest".into(),
                    output: "test.v1.EchoResponse".into(),
                    client_streaming: false,
                    server_streaming: false,
                },
                GrpcMethodInfo {
                    service: "test.v1.Echo".into(),
                    method: "StreamOut".into(),
                    input: "test.v1.EchoRequest".into(),
                    output: "test.v1.EchoResponse".into(),
                    client_streaming: false,
                    server_streaming: true,
                },
                GrpcMethodInfo {
                    service: "test.v1.Echo".into(),
                    method: "StreamIn".into(),
                    input: "test.v1.EchoRequest".into(),
                    output: "test.v1.EchoResponse".into(),
                    client_streaming: true,
                    server_streaming: false,
                },
            ]
        );
    }

    #[test]
    fn empty_proto_list_fails_loud() {
        assert!(compile(&[])
            .unwrap_err()
            .to_string()
            .contains("no proto files"));
    }

    #[test]
    fn json_to_dynamic_message_roundtrips() {
        let (_dir, path) = write_proto();
        let pool = compile(&[path]).unwrap();
        let desc = pool.get_message_by_name("test.v1.EchoRequest").unwrap();
        let msg = parse_message(desc, r#"{"text": "hola", "count": 3}"#).unwrap();
        let back = serde_json::to_value(&msg).unwrap();
        assert_eq!(back, serde_json::json!({"text": "hola", "count": 3}));
    }

    #[test]
    fn invalid_message_json_fails_loud() {
        let (_dir, path) = write_proto();
        let pool = compile(&[path]).unwrap();
        let desc = pool.get_message_by_name("test.v1.EchoRequest").unwrap();
        let err = parse_message(desc, "not json").unwrap_err().to_string();
        assert!(err.contains("invalid message JSON"));
    }

    #[test]
    fn streaming_methods_fail_loud() {
        let (_dir, path) = write_proto();
        let pool = compile(&[path]).unwrap();
        for name in ["StreamOut", "StreamIn"] {
            let method = find_method(&pool, "test.v1.Echo", name).unwrap();
            assert_eq!(
                ensure_unary(&method).unwrap_err().to_string(),
                "streaming methods not supported yet"
            );
        }
        let unary = find_method(&pool, "test.v1.Echo", "Say").unwrap();
        assert!(ensure_unary(&unary).is_ok());
    }

    #[test]
    fn redacts_userinfo_in_urls() {
        assert_eq!(
            redact_url("https://user:pass@x.dev:443/svc"),
            "https://<redacted>@x.dev:443/svc"
        );
        assert_eq!(redact_url("http://x.dev:50051"), "http://x.dev:50051");
        assert_eq!(redact_url("nonsense"), "nonsense");
    }

    #[test]
    fn metadata_value_error_does_not_echo_the_value() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, path) = write_proto();
        let spec: GrpcSpec = serde_json::from_value(serde_json::json!({
            "url": "http://127.0.0.1:1",
            "protoPaths": [path],
            "service": "test.v1.Echo",
            "method": "Say",
            "message": "{}",
            "metadata": [["x-secret", "top\u{7f}secret"]]
        }))
        .unwrap();
        let err = rt.block_on(send_grpc(spec)).unwrap_err().to_string();
        assert_eq!(err, "invalid metadata value for key x-secret (ascii only)");
    }

    #[test]
    fn unknown_service_and_method_fail_loud() {
        let (_dir, path) = write_proto();
        let pool = compile(&[path]).unwrap();
        assert!(find_method(&pool, "test.v1.Nope", "Say")
            .unwrap_err()
            .to_string()
            .contains("service not found"));
        assert!(find_method(&pool, "test.v1.Echo", "Nope")
            .unwrap_err()
            .to_string()
            .contains("method not found"));
    }
}
