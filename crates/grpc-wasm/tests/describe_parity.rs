//! The browser proto compiler and the desktop one carry two copies of the same
//! walk, and a shape that differs between them is a bug the user meets as "the
//! example is wrong in the web app". Both copies are held to one golden file:
//! this suite compiles the fixture from memory, `grpc_describe.rs` over in
//! `mandalo-core` compiles the same fixture from disk, and both assert the same
//! JSON. The crates stay independent — only the fixture is shared.

use mandalo_grpc_wasm::{describe, ProtoFile};
use serde_json::Value;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../core/tests/fixtures/describe"
);

fn files() -> Vec<ProtoFile> {
    ["parity.proto", "parity_common.proto"]
        .iter()
        .map(|name| ProtoFile {
            path: format!("protos/{name}"),
            contents: std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap(),
        })
        .collect()
}

fn golden() -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURES}/parity.json")).unwrap())
        .unwrap()
}

fn shape(type_name: &str) -> Value {
    serde_json::to_value(describe(&files(), type_name).unwrap()).unwrap()
}

#[test]
fn the_browser_build_produces_the_golden_shape() {
    assert_eq!(shape("fix.v1.Everything"), golden());
}

#[test]
fn imports_and_well_known_types_resolve_without_a_filesystem() {
    let meta = shape("fix.v1.Meta");
    assert_eq!(
        meta,
        serde_json::json!({
            "name": "fix.v1.Meta",
            "fields": [
                {"name": "trace", "type": "string", "repeated": false, "message": null, "enumValues": []},
                {"name": "hops", "type": "string", "repeated": true, "message": null, "enumValues": []}
            ]
        })
    );
}

#[test]
fn a_self_referential_message_terminates_in_the_browser_too() {
    let node = shape("fix.v1.Node");
    let child = &node["fields"][1];
    assert_eq!(child["name"], "child");
    assert_eq!(child["type"], "message");
    assert_eq!(child["message"], Value::Null);
    assert_eq!(node["fields"][2]["message"], Value::Null);
}

#[test]
fn an_unknown_type_names_it_and_lists_what_the_protos_do_define() {
    let err = describe(&files(), "fix.v1.Nope").unwrap_err();
    assert_eq!(
        err,
        "message type not found: fix.v1.Nope. These protos define: fix.v1.Meta, fix.v1.Node, fix.v1.Everything"
    );
}

#[test]
fn no_proto_files_still_fails_before_the_lookup() {
    assert_eq!(
        describe(&[], "fix.v1.Meta").unwrap_err(),
        "no proto files given"
    );
}
