use crate::interpolate;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, MethodDescriptor};
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcSpec {
    pub url: String,
    pub proto_paths: Vec<String>,
    pub service: String,
    pub method: String,
    pub message: String,
    #[serde(default)]
    pub metadata: Vec<(String, String)>,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GrpcResponse {
    pub body: String,
    pub duration_ms: u128,
}

pub fn compile(proto_paths: &[String]) -> Result<DescriptorPool, String> {
    if proto_paths.is_empty() {
        return Err("no proto files given".to_string());
    }
    let mut includes: Vec<PathBuf> = Vec::new();
    for p in proto_paths {
        let parent = Path::new(p)
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .ok_or_else(|| format!("proto path has no parent directory: {p}"))?
            .to_path_buf();
        if !includes.contains(&parent) {
            includes.push(parent);
        }
    }
    let set = protox::compile(proto_paths, &includes).map_err(|e| e.to_string())?;
    DescriptorPool::from_file_descriptor_set(set).map_err(|e| e.to_string())
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

#[tauri::command]
pub async fn list_grpc_methods(proto_paths: Vec<String>) -> Result<Vec<GrpcMethodInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || grpc_methods(&proto_paths))
        .await
        .map_err(|e| format!("proto compilation task failed: {e}"))?
}

pub fn grpc_methods(proto_paths: &[String]) -> Result<Vec<GrpcMethodInfo>, String> {
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

#[tauri::command]
pub async fn send_grpc(spec: GrpcSpec) -> Result<GrpcResponse, String> {
    let vars = &spec.vars;
    let url = interpolate::apply(&spec.url, vars)?;
    let message = interpolate::apply(&spec.message, vars)?;
    let mut metadata = Vec::with_capacity(spec.metadata.len());
    for (k, v) in &spec.metadata {
        metadata.push((k.clone(), interpolate::apply(v, vars)?));
    }

    let pool = compile(&spec.proto_paths)?;
    let method = find_method(&pool, &spec.service, &spec.method)?;
    ensure_unary(&method)?;
    let request_message = parse_message(method.input(), &message)?;

    let mut request = tonic::Request::new(request_message);
    for (k, v) in metadata {
        let key: tonic::metadata::MetadataKey<tonic::metadata::Ascii> = k
            .parse()
            .map_err(|_| format!("invalid metadata key (ascii only): {k}"))?;
        let value: tonic::metadata::AsciiMetadataValue = v
            .parse()
            .map_err(|_| format!("invalid metadata value for key {k} (ascii only)"))?;
        request.metadata_mut().insert(key, value);
    }

    let endpoint = if url.starts_with("http://") {
        tonic::transport::Endpoint::from_shared(url.clone()).map_err(|e| e.to_string())?
    } else if url.starts_with("https://") {
        tonic::transport::Endpoint::from_shared(url.clone())
            .map_err(|e| e.to_string())?
            .tls_config(tonic::transport::ClientTlsConfig::new().with_native_roots())
            .map_err(|e| e.to_string())?
    } else {
        return Err(format!(
            "grpc url must start with http:// or https://: {}",
            redact_url(&url)
        ));
    };
    let endpoint = endpoint
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(CALL_TIMEOUT);

    let channel = endpoint
        .connect()
        .await
        .map_err(|e| format!("failed to connect to {}: {e}", redact_url(&url)))?;
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready().await.map_err(|e| e.to_string())?;

    let path: http::uri::PathAndQuery = format!("/{}/{}", spec.service, spec.method)
        .parse()
        .map_err(|e: http::uri::InvalidUri| e.to_string())?;

    let codec = DynamicCodec::new(method.output());
    let started = Instant::now();
    let response = grpc
        .unary(request, path, codec)
        .await
        .map_err(|s| format!("grpc error {:?}: {}", s.code(), s.message()))?;
    let body =
        serde_json::to_string_pretty(response.get_ref()).map_err(|e| e.to_string())?;
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
        assert!(compile(&[]).unwrap_err().contains("no proto files"));
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
        let err = parse_message(desc, "not json").unwrap_err();
        assert!(err.contains("invalid message JSON"));
    }

    #[test]
    fn streaming_methods_fail_loud() {
        let (_dir, path) = write_proto();
        let pool = compile(&[path]).unwrap();
        for name in ["StreamOut", "StreamIn"] {
            let method = find_method(&pool, "test.v1.Echo", name).unwrap();
            assert_eq!(
                ensure_unary(&method).unwrap_err(),
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
        let err = rt.block_on(send_grpc(spec)).unwrap_err();
        assert_eq!(err, "invalid metadata value for key x-secret (ascii only)");
    }

    #[test]
    fn unknown_service_and_method_fail_loud() {
        let (_dir, path) = write_proto();
        let pool = compile(&[path]).unwrap();
        assert!(find_method(&pool, "test.v1.Nope", "Say")
            .unwrap_err()
            .contains("service not found"));
        assert!(find_method(&pool, "test.v1.Echo", "Nope")
            .unwrap_err()
            .contains("method not found"));
    }
}
