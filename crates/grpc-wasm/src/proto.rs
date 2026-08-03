use prost::Message as _;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, MethodDescriptor,
};
use protox::file::{ChainFileResolver, File, FileResolver, GoogleFileResolver};
use protox::{Compiler, Error as ProtoxError};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProtoFile {
    pub path: String,
    pub contents: String,
}

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

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(at) => &path[..at],
        None => "",
    }
}

fn includes_of(files: &[ProtoFile]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for file in files {
        let dir = parent_of(&file.path).to_string();
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// An in-memory `IncludeFileResolver`: a proto's unique name is its path with the
/// include directory stripped, exactly as on disk, so `protos/echo.proto` and an
/// `import "common.proto"` next to it agree on one name per file. Naming the same
/// file twice is what protoc calls shadowing and it fails as a duplicate definition.
struct MemoryResolver {
    files: Vec<ProtoFile>,
    includes: Vec<String>,
}

impl MemoryResolver {
    fn key(&self, include: &str, name: &str) -> String {
        if include.is_empty() {
            name.to_string()
        } else {
            format!("{include}/{name}")
        }
    }
}

impl FileResolver for MemoryResolver {
    fn resolve_path(&self, path: &Path) -> Option<String> {
        let wanted = path.to_str()?;
        if !self.files.iter().any(|f| f.path == wanted) {
            return None;
        }
        for include in &self.includes {
            if include.is_empty() {
                if !wanted.contains('/') {
                    return Some(wanted.to_string());
                }
            } else if let Some(rest) = wanted.strip_prefix(&format!("{include}/")) {
                return Some(rest.to_string());
            }
        }
        None
    }

    fn open_file(&self, name: &str) -> Result<File, ProtoxError> {
        for include in &self.includes {
            let key = self.key(include, name);
            if let Some(found) = self.files.iter().find(|f| f.path == key) {
                return File::from_source(name, &found.contents);
            }
        }
        Err(ProtoxError::file_not_found(name))
    }
}

pub fn compile(files: &[ProtoFile]) -> Result<DescriptorPool, String> {
    if files.is_empty() {
        return Err("no proto files given".to_string());
    }
    let mut resolver = ChainFileResolver::new();
    resolver.add(MemoryResolver {
        files: files.to_vec(),
        includes: includes_of(files),
    });
    resolver.add(GoogleFileResolver::new());

    let mut compiler = Compiler::with_file_resolver(resolver);
    for file in files {
        compiler
            .open_file(&file.path)
            .map_err(|e| format!("{}: {e}", file.path))?;
    }
    Ok(compiler.descriptor_pool())
}

pub fn methods(files: &[ProtoFile]) -> Result<Vec<GrpcMethodInfo>, String> {
    let pool = compile(files)?;
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

fn unknown_type(pool: &DescriptorPool, type_name: &str) -> String {
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
    format!("message type not found: {type_name}. These protos {defines}")
}

pub fn describe(files: &[ProtoFile], type_name: &str) -> Result<MessageShape, String> {
    let pool = compile(files)?;
    let descriptor = pool
        .get_message_by_name(type_name)
        .ok_or_else(|| unknown_type(&pool, type_name))?;
    let mut walk = Walk {
        path: Vec::new(),
        budget: MAX_NODES,
    };
    Ok(walk.message(&descriptor, 0))
}

fn find_method(
    pool: &DescriptorPool,
    service: &str,
    method: &str,
) -> Result<MethodDescriptor, String> {
    let svc = pool
        .get_service_by_name(service)
        .ok_or_else(|| format!("service not found: {service}"))?;
    let found = svc.methods().find(|m| m.name() == method);
    found.ok_or_else(|| format!("method not found: {service}/{method}"))
}

fn ensure_unary(method: &MethodDescriptor) -> Result<(), String> {
    if method.is_client_streaming() || method.is_server_streaming() {
        return Err("streaming methods not supported yet".to_string());
    }
    Ok(())
}

fn parse_message(descriptor: MessageDescriptor, json: &str) -> Result<DynamicMessage, String> {
    let mut de = serde_json::Deserializer::from_str(json);
    let msg = DynamicMessage::deserialize(descriptor, &mut de)
        .map_err(|e| format!("invalid message JSON: {e}"))?;
    de.end().map_err(|e| format!("invalid message JSON: {e}"))?;
    Ok(msg)
}

pub fn encode(
    files: &[ProtoFile],
    service: &str,
    method: &str,
    json: &str,
) -> Result<Vec<u8>, String> {
    let pool = compile(files)?;
    let method = find_method(&pool, service, method)?;
    ensure_unary(&method)?;
    Ok(parse_message(method.input(), json)?.encode_to_vec())
}

pub fn decode(
    files: &[ProtoFile],
    service: &str,
    method: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let pool = compile(files)?;
    let method = find_method(&pool, service, method)?;
    ensure_unary(&method)?;
    let msg = DynamicMessage::decode(method.output(), bytes).map_err(|e| {
        format!(
            "could not decode the response as {}: {e}",
            method.output().full_name()
        )
    })?;
    serde_json::to_string_pretty(&msg).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ECHO: &str = r#"
syntax = "proto3";
package test.v1;

import "common.proto";

message EchoRequest {
  string text = 1;
  int32 count = 2;
  test.v1.Meta meta = 3;
}

message EchoResponse {
  string text = 1;
  int32 doubled = 2;
}

service Echo {
  rpc Say(EchoRequest) returns (EchoResponse);
  rpc StreamOut(EchoRequest) returns (stream EchoResponse);
  rpc StreamIn(stream EchoRequest) returns (EchoResponse);
}
"#;

    const COMMON: &str = r#"
syntax = "proto3";
package test.v1;

message Meta {
  string trace = 1;
}
"#;

    fn files() -> Vec<ProtoFile> {
        vec![
            ProtoFile {
                path: "protos/echo.proto".to_string(),
                contents: ECHO.to_string(),
            },
            ProtoFile {
                path: "protos/common.proto".to_string(),
                contents: COMMON.to_string(),
            },
        ]
    }

    #[test]
    fn compiles_from_memory_and_lists_methods_with_streaming_flags() {
        assert_eq!(
            methods(&files()).unwrap(),
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
    fn imports_resolve_from_the_include_directory_without_a_filesystem() {
        let pool = compile(&files()).unwrap();
        assert!(pool.get_message_by_name("test.v1.Meta").is_some());
    }

    #[test]
    fn well_known_imports_resolve() {
        let files = vec![ProtoFile {
            path: "wk.proto".to_string(),
            contents: r#"
syntax = "proto3";
package wk.v1;
import "google/protobuf/timestamp.proto";
message At { google.protobuf.Timestamp when = 1; }
service Clock { rpc Now(At) returns (At); }
"#
            .to_string(),
        }];
        assert_eq!(methods(&files).unwrap().len(), 1);
    }

    #[test]
    fn encode_decode_roundtrips_through_protobuf_bytes() {
        let bytes = encode(
            &files(),
            "test.v1.Echo",
            "Say",
            r#"{"text": "hola", "count": 21}"#,
        )
        .unwrap();
        assert!(!bytes.is_empty());

        let json = decode(&files(), "test.v1.Echo", "Say", &bytes).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back, serde_json::json!({"text": "hola", "doubled": 21}));
    }

    #[test]
    fn empty_message_encodes_to_zero_bytes() {
        assert_eq!(
            encode(&files(), "test.v1.Echo", "Say", "{}").unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn empty_file_list_fails_loud() {
        assert_eq!(methods(&[]).unwrap_err(), "no proto files given");
    }

    #[test]
    fn missing_import_fails_loud() {
        let files = vec![ProtoFile {
            path: "protos/echo.proto".to_string(),
            contents: ECHO.to_string(),
        }];
        let err = methods(&files).unwrap_err();
        assert!(err.contains("common.proto"), "{err}");
    }

    #[test]
    fn invalid_proto_syntax_fails_loud() {
        let files = vec![ProtoFile {
            path: "bad.proto".to_string(),
            contents: "this is not a proto file".to_string(),
        }];
        assert!(methods(&files).unwrap_err().contains("bad.proto"));
    }

    #[test]
    fn streaming_methods_fail_loud() {
        for name in ["StreamOut", "StreamIn"] {
            assert_eq!(
                encode(&files(), "test.v1.Echo", name, "{}").unwrap_err(),
                "streaming methods not supported yet"
            );
        }
    }

    #[test]
    fn unknown_service_and_method_fail_loud() {
        assert_eq!(
            encode(&files(), "test.v1.Nope", "Say", "{}").unwrap_err(),
            "service not found: test.v1.Nope"
        );
        assert_eq!(
            encode(&files(), "test.v1.Echo", "Nope", "{}").unwrap_err(),
            "method not found: test.v1.Echo/Nope"
        );
    }

    #[test]
    fn invalid_message_json_fails_loud() {
        let err = encode(&files(), "test.v1.Echo", "Say", "not json").unwrap_err();
        assert!(err.starts_with("invalid message JSON"), "{err}");
    }

    #[test]
    fn unknown_field_fails_loud() {
        let err = encode(&files(), "test.v1.Echo", "Say", r#"{"nope": 1}"#).unwrap_err();
        assert!(err.starts_with("invalid message JSON"), "{err}");
    }

    #[test]
    fn undecodable_response_bytes_fail_loud() {
        let err = decode(&files(), "test.v1.Echo", "Say", &[0xff, 0xff, 0xff]).unwrap_err();
        assert!(
            err.contains("could not decode the response as test.v1.EchoResponse"),
            "{err}"
        );
    }
}
