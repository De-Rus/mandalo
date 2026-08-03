use mandalo_core::error::CoreError;
use mandalo_core::grpc::{compile, message_shape, MessageShape, ProtoField, ProtoFieldType};
use prost_reflect::{DynamicMessage, MessageDescriptor};
use serde_json::{json, Value};

fn write(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for (name, contents) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        paths.push(path.to_str().unwrap().to_string());
    }
    (dir, paths)
}

fn shape(proto: &str, type_name: &str) -> MessageShape {
    let (_dir, paths) = write(&[("t.proto", proto)]);
    message_shape(&paths, type_name).unwrap()
}

fn field<'a>(shape: &'a MessageShape, name: &str) -> &'a ProtoField {
    shape
        .fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name} in {}", shape.name))
}

/// The literal port of `src/lib/skeleton.ts` — the example the editor writes into
/// the message box. Every shape this crate produces has to survive it.
fn placeholder(field: &ProtoField) -> Value {
    if field.repeated {
        return json!([]);
    }
    match field.field_type {
        ProtoFieldType::Message => match &field.message {
            Some(nested) => shape_value(nested),
            None => json!({}),
        },
        ProtoFieldType::Enum => json!(field.enum_values.first().cloned().unwrap_or_default()),
        ProtoFieldType::Bool => json!(false),
        ProtoFieldType::Number => json!(0),
        _ => json!(""),
    }
}

fn shape_value(shape: &MessageShape) -> Value {
    Value::Object(
        shape
            .fields
            .iter()
            .map(|f| (f.name.clone(), placeholder(f)))
            .collect(),
    )
}

fn encodes(descriptor: MessageDescriptor, skeleton: &Value) -> Result<(), String> {
    let text = serde_json::to_string(skeleton).unwrap();
    let mut de = serde_json::Deserializer::from_str(&text);
    DynamicMessage::deserialize(descriptor, &mut de)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn assert_skeleton_encodes(proto: &str, type_name: &str) {
    let (_dir, paths) = write(&[("t.proto", proto)]);
    let shape = message_shape(&paths, type_name).unwrap();
    let pool = compile(&paths).unwrap();
    let descriptor = pool.get_message_by_name(type_name).unwrap();
    if let Err(e) = encodes(descriptor, &shape_value(&shape)) {
        panic!("the example for {type_name} is not a message the encoder accepts: {e}");
    }
}

const SCALARS: &str = r#"
syntax = "proto3";
package t.v1;
message Every {
  double a = 1;
  float b = 2;
  int32 c = 3;
  int64 d = 4;
  uint32 e = 5;
  uint64 f = 6;
  sint32 g = 7;
  sint64 h = 8;
  fixed32 i = 9;
  fixed64 j = 10;
  sfixed32 k = 11;
  sfixed64 l = 12;
  bool m = 13;
  string n = 14;
  bytes o = 15;
}
"#;

#[test]
fn every_scalar_kind_maps_to_one_of_the_four_json_shapes() {
    let shape = shape(SCALARS, "t.v1.Every");
    assert_eq!(shape.name, "t.v1.Every");
    let types: Vec<ProtoFieldType> = shape.fields.iter().map(|f| f.field_type).collect();
    let numbers = std::iter::repeat_n(ProtoFieldType::Number, 12);
    let expected: Vec<ProtoFieldType> = numbers
        .chain([
            ProtoFieldType::Bool,
            ProtoFieldType::String,
            ProtoFieldType::Bytes,
        ])
        .collect();
    assert_eq!(types, expected);
    assert!(shape.fields.iter().all(|f| !f.repeated));
    assert!(shape.fields.iter().all(|f| f.message.is_none()));
    assert!(shape.fields.iter().all(|f| f.enum_values.is_empty()));
    assert_skeleton_encodes(SCALARS, "t.v1.Every");
}

#[test]
fn field_names_stay_as_written_in_the_proto() {
    let shape = shape(
        r#"
syntax = "proto3";
package t.v1;
message Snake { string user_id = 1; int32 retry_count_max = 2; }
"#,
        "t.v1.Snake",
    );
    let names: Vec<&str> = shape.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["user_id", "retry_count_max"]);
}

const NESTED: &str = r#"
syntax = "proto3";
package t.v1;
enum Tier {
  TIER_UNSPECIFIED = 0;
  TIER_FREE = 1;
  TIER_PRO = 2;
}
message Address { string city = 1; }
message User {
  repeated string tags = 1;
  repeated Address homes = 2;
  Address main = 3;
  Tier tier = 4;
  repeated Tier history = 5;
}
"#;

#[test]
fn repeated_is_flagged_for_scalars_messages_and_enums() {
    let shape = shape(NESTED, "t.v1.User");
    assert!(field(&shape, "tags").repeated);
    assert!(field(&shape, "homes").repeated);
    assert!(!field(&shape, "main").repeated);
    assert!(field(&shape, "history").repeated);
}

#[test]
fn a_nested_message_carries_its_own_shape() {
    let shape = shape(NESTED, "t.v1.User");
    let main = field(&shape, "main");
    assert_eq!(main.field_type, ProtoFieldType::Message);
    let inner = main.message.as_ref().unwrap();
    assert_eq!(inner.name, "t.v1.Address");
    assert_eq!(inner.fields.len(), 1);
    assert_eq!(inner.fields[0].name, "city");
    assert_eq!(inner.fields[0].field_type, ProtoFieldType::String);
}

#[test]
fn an_enum_lists_its_values_in_declaration_order() {
    let shape = shape(NESTED, "t.v1.User");
    let tier = field(&shape, "tier");
    assert_eq!(tier.field_type, ProtoFieldType::Enum);
    assert_eq!(
        tier.enum_values,
        vec!["TIER_UNSPECIFIED", "TIER_FREE", "TIER_PRO"]
    );
    assert!(tier.message.is_none());
    assert_skeleton_encodes(NESTED, "t.v1.User");
}

const SELF_REF: &str = r#"
syntax = "proto3";
package t.v1;
message Node {
  string label = 1;
  Node child = 2;
  repeated Node children = 3;
}
"#;

#[test]
fn a_self_referential_message_stops_at_the_repeat_instead_of_expanding_forever() {
    let shape = shape(SELF_REF, "t.v1.Node");
    for name in ["child", "children"] {
        let f = field(&shape, name);
        assert_eq!(f.field_type, ProtoFieldType::Message);
        assert!(
            f.message.is_none(),
            "{name} expanded t.v1.Node inside itself"
        );
    }
    assert_skeleton_encodes(SELF_REF, "t.v1.Node");
}

#[test]
fn a_cycle_through_two_types_also_stops() {
    let shape = shape(
        r#"
syntax = "proto3";
package t.v1;
message A { B b = 1; }
message B { A a = 1; }
"#,
        "t.v1.A",
    );
    let b = field(&shape, "b").message.as_ref().unwrap();
    assert_eq!(b.name, "t.v1.B");
    assert!(field(b, "a").message.is_none());
}

fn count(shape: &MessageShape) -> usize {
    1 + shape
        .fields
        .iter()
        .filter_map(|f| f.message.as_ref())
        .map(count)
        .sum::<usize>()
}

fn depth_of(shape: &MessageShape) -> usize {
    1 + shape
        .fields
        .iter()
        .filter_map(|f| f.message.as_ref())
        .map(depth_of)
        .max()
        .unwrap_or(0)
}

#[test]
fn a_deep_chain_is_cut_at_the_depth_limit() {
    let mut proto = String::from("syntax = \"proto3\";\npackage t.v1;\n");
    for i in 0..20 {
        proto.push_str(&format!("message D{i} {{ D{} next = 1; }}\n", i + 1));
    }
    proto.push_str("message D20 { string end = 1; }\n");

    let shape = shape(&proto, "t.v1.D0");
    assert_eq!(depth_of(&shape), 9, "root plus eight expanded levels");
    let mut node = &shape;
    for _ in 0..8 {
        node = node.fields[0].message.as_ref().unwrap();
    }
    assert!(
        node.fields[0].message.is_none(),
        "the cut is a null message"
    );
}

#[test]
fn a_wide_tree_is_cut_by_the_node_budget() {
    let mut proto = String::from("syntax = \"proto3\";\npackage t.v1;\n");
    for level in 0..8 {
        let fields: String = (1..=8)
            .map(|n| format!("  W{} f{n} = {n};\n", level + 1))
            .collect();
        proto.push_str(&format!("message W{level} {{\n{fields}}}\n"));
    }
    proto.push_str("message W8 { string end = 1; }\n");

    let shape = shape(&proto, "t.v1.W0");
    let nodes = count(&shape);
    assert!(nodes <= 2_001, "node budget did not hold: {nodes}");
    assert!(nodes > 1_000, "the budget cut far too early: {nodes}");
    let text = serde_json::to_string(&shape).unwrap();
    assert!(text.len() < 400_000, "shape json is {} bytes", text.len());
}

const MAPS: &str = r#"
syntax = "proto3";
package t.v1;
message Address { string city = 1; }
message Labelled {
  map<string, int32> counts = 1;
  map<string, Address> homes = 2;
}
"#;

#[test]
fn a_map_is_one_object_field_not_a_repeated_entry_message() {
    let shape = shape(MAPS, "t.v1.Labelled");
    let counts = field(&shape, "counts");
    assert_eq!(counts.field_type, ProtoFieldType::Message);
    assert!(!counts.repeated, "a map is written as a JSON object");
    let entry = counts.message.as_ref().unwrap();
    assert_eq!(entry.name, "map<string, int32>");
    assert!(entry.fields.is_empty(), "the synthetic entry never leaks");

    let homes = field(&shape, "homes").message.as_ref().unwrap();
    assert_eq!(homes.name, "map<string, t.v1.Address>");
    assert_skeleton_encodes(MAPS, "t.v1.Labelled");
}

const ONEOF: &str = r#"
syntax = "proto3";
package t.v1;
message Payload {
  string id = 1;
  oneof body {
    string text = 2;
    int32 number = 3;
  }
  optional string note = 4;
}
"#;

#[test]
fn oneof_members_are_listed_flat_and_optional_is_an_ordinary_field() {
    let shape = shape(ONEOF, "t.v1.Payload");
    let names: Vec<&str> = shape.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["id", "text", "number", "note"]);
    assert_eq!(field(&shape, "text").field_type, ProtoFieldType::String);
    assert_eq!(field(&shape, "number").field_type, ProtoFieldType::Number);
}

/// KNOWN GAP: `MessageShape` has no way to say "pick one of these", so the example
/// for a oneof sets every arm and the encoder refuses it until the user deletes the
/// ones they do not want. Listing every arm is still the better half of the trade:
/// `unknownFields()` would otherwise flag whichever arm the user chose. Closing this
/// needs a `oneof` group on the TS `ProtoField`.
#[test]
fn a_oneof_example_holds_every_arm_and_so_needs_an_edit_before_it_encodes() {
    let (_dir, paths) = write(&[("t.proto", ONEOF)]);
    let shape = message_shape(&paths, "t.v1.Payload").unwrap();
    let pool = compile(&paths).unwrap();
    let descriptor = pool.get_message_by_name("t.v1.Payload").unwrap();
    let err = encodes(descriptor.clone(), &shape_value(&shape)).unwrap_err();
    assert!(err.contains("multiple fields provided for oneof"), "{err}");

    let mut kept = shape_value(&shape);
    kept.as_object_mut().unwrap().remove("number");
    assert!(encodes(descriptor, &kept).is_ok());
}

#[test]
fn proto3_optional_alone_does_not_trip_the_oneof_rule() {
    let proto = r#"
syntax = "proto3";
package t.v1;
message Note { optional string body = 1; optional int32 tries = 2; }
"#;
    let shape = shape(proto, "t.v1.Note");
    let names: Vec<&str> = shape.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["body", "tries"]);
    assert_skeleton_encodes(proto, "t.v1.Note");
}

const WELL_KNOWN: &str = r#"
syntax = "proto3";
package t.v1;
import "google/protobuf/timestamp.proto";
import "google/protobuf/duration.proto";
import "google/protobuf/wrappers.proto";
import "google/protobuf/struct.proto";
import "google/protobuf/empty.proto";
message Meta {
  google.protobuf.Timestamp at = 1;
  google.protobuf.Duration took = 2;
  google.protobuf.StringValue label = 3;
  google.protobuf.Int32Value tries = 4;
  google.protobuf.BoolValue live = 5;
  google.protobuf.Struct extra = 6;
  google.protobuf.ListValue items = 7;
  google.protobuf.Empty nothing = 8;
}
"#;

#[test]
fn well_known_types_are_described_as_the_json_they_actually_take() {
    let shape = shape(WELL_KNOWN, "t.v1.Meta");
    assert_eq!(field(&shape, "at").field_type, ProtoFieldType::String);
    assert_eq!(field(&shape, "took").field_type, ProtoFieldType::String);
    assert_eq!(field(&shape, "label").field_type, ProtoFieldType::String);
    assert_eq!(field(&shape, "tries").field_type, ProtoFieldType::Number);
    assert_eq!(field(&shape, "live").field_type, ProtoFieldType::Bool);
    assert_eq!(field(&shape, "extra").field_type, ProtoFieldType::Message);
    assert!(field(&shape, "extra").message.is_none());
    assert!(
        field(&shape, "items").repeated,
        "a ListValue is a JSON array"
    );
    assert!(field(&shape, "nothing").message.is_none());
}

#[test]
fn an_unknown_type_names_it_and_lists_what_the_protos_do_define() {
    let (_dir, paths) = write(&[("t.proto", NESTED)]);
    let err = message_shape(&paths, "t.v1.Nope").unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
    let text = err.to_string();
    assert!(text.contains("message type not found: t.v1.Nope"), "{text}");
    assert!(text.contains("t.v1.Address"), "{text}");
    assert!(text.contains("t.v1.User"), "{text}");
}

#[test]
fn the_list_of_known_types_is_bounded() {
    let mut proto = String::from("syntax = \"proto3\";\npackage t.v1;\n");
    for i in 0..25 {
        proto.push_str(&format!("message M{i} {{ string a = 1; }}\n"));
    }
    let (_dir, paths) = write(&[("t.proto", &proto)]);
    let text = message_shape(&paths, "t.v1.Nope").unwrap_err().to_string();
    assert!(text.contains("…and 15 more"), "{text}");
    assert!(!text.contains("t.v1.M10"), "{text}");
}

#[test]
fn imports_resolve_through_the_include_directory() {
    let (_dir, paths) = write(&[
        (
            "main.proto",
            r#"
syntax = "proto3";
package t.v1;
import "common.proto";
message Wrapper { t.v1.Meta meta = 1; }
"#,
        ),
        (
            "common.proto",
            r#"
syntax = "proto3";
package t.v1;
message Meta { string trace = 1; }
"#,
        ),
    ]);
    let shape = message_shape(&paths, "t.v1.Wrapper").unwrap();
    let meta = field(&shape, "meta").message.as_ref().unwrap();
    assert_eq!(meta.name, "t.v1.Meta");
    assert_eq!(meta.fields[0].name, "trace");
}

/// The other half of the parity pin: `mandalo-grpc-wasm` compiles these same two
/// files from memory and asserts the same golden. If the browser walk and this one
/// ever drift, one of the two suites goes red.
#[test]
fn the_desktop_build_produces_the_golden_shape() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/describe");
    let paths = vec![
        format!("{dir}/parity.proto"),
        format!("{dir}/parity_common.proto"),
    ];
    let shape = message_shape(&paths, "fix.v1.Everything").unwrap();
    let golden: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{dir}/parity.json")).unwrap())
            .unwrap();
    assert_eq!(serde_json::to_value(&shape).unwrap(), golden);
}

#[test]
fn the_mock_workspace_get_user_request_serializes_as_the_frontend_expects() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/mock-workspace/protos/mock.proto"
    );
    let shape = message_shape(&[path.to_string()], "mock.v1.GetUserRequest").unwrap();
    assert_eq!(
        serde_json::to_value(&shape).unwrap(),
        json!({
            "name": "mock.v1.GetUserRequest",
            "fields": [
                {"name": "id", "type": "string", "repeated": false, "message": null, "enumValues": []},
                {"name": "tags", "type": "string", "repeated": true, "message": null, "enumValues": []},
                {"name": "tier", "type": "enum", "repeated": false, "message": null,
                 "enumValues": ["TIER_UNSPECIFIED", "TIER_FREE", "TIER_PRO"]}
            ]
        })
    );
    assert_eq!(
        shape_value(&shape),
        json!({"id": "", "tags": [], "tier": "TIER_UNSPECIFIED"})
    );
}

#[test]
fn the_mock_workspace_response_expands_its_nested_user() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/mock-workspace/protos/mock.proto"
    );
    let shape = message_shape(&[path.to_string()], "mock.v1.GetUserResponse").unwrap();
    let user = field(&shape, "user").message.as_ref().unwrap();
    assert_eq!(user.name, "mock.v1.User");
    let address = field(user, "address").message.as_ref().unwrap();
    assert_eq!(address.name, "mock.v1.Address");
}
