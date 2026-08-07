//! What an OpenAPI import is worth is not whether it parses — it is whether the
//! collection it writes runs. These tests import hand-written specs, read the
//! `.http` files back off disk, and send the result at the testkit mock.

use mandalo_core::body::FormDataRow;
use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::postman::ImportReport;
use mandalo_core::request::Auth;
use mandalo_core::workspace;
use mandalo_core::{openapi, Body, RawLanguage};
use mandalo_testkit::fixtures;
use mandalo_testkit::MockApi;
use std::path::Path;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openapi")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn collect(
    workspace: &Path,
    slug: &str,
    folders: &[collection::FolderNode],
    summaries: &[collection::RequestSummary],
    out: &mut Vec<(String, SavedRequest)>,
) {
    for summary in summaries {
        out.push((
            summary.path.clone(),
            collection::load_request_source(workspace, slug, &summary.path).unwrap(),
        ));
    }
    for folder in folders {
        collect(workspace, slug, &folder.folders, &folder.requests, out);
    }
}

fn imported(workspace: &Path) -> Vec<(String, SavedRequest)> {
    let mut out = Vec::new();
    for node in collection::list_tree(workspace).unwrap().collections {
        collect(
            workspace,
            &node.slug,
            &node.folders,
            &node.requests,
            &mut out,
        );
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn by_name<'a>(requests: &'a [(String, SavedRequest)], name: &str) -> &'a SavedRequest {
    &requests
        .iter()
        .find(|(_, r)| r.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no request named {name:?}; imported: {:?}",
                requests.iter().map(|(_, r)| &r.name).collect::<Vec<_>>()
            )
        })
        .1
}

fn path_of(requests: &[(String, SavedRequest)], name: &str) -> String {
    requests
        .iter()
        .find(|(_, r)| r.name == name)
        .unwrap()
        .0
        .clone()
}

fn file_text(workspace: &Path, slug: &str, file: &str) -> String {
    let path = collection::collections_dir(workspace).join(slug).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn warning_with(report: &ImportReport, needle: &str) -> String {
    report
        .warnings
        .iter()
        .find(|w| w.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "no warning mentioning {needle:?}; warnings: {:?}",
                report.warnings
            )
        })
        .clone()
}

fn env_named(workspace: &Path, name: &str) -> std::collections::BTreeMap<String, String> {
    workspace::list_env_docs(workspace)
        .unwrap()
        .items
        .into_iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no environment named {name:?}"))
        .shared_view()
        .vars
}

#[test]
fn a_3_0_spec_becomes_a_collection_a_human_recognises() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("petstore-3.0.json")).unwrap();

    assert_eq!(report.imported, 7);
    assert_eq!(report.collections, 1);
    assert_eq!(report.environments, 2);

    let requests = imported(dir.path());
    assert_eq!(requests.len(), 7);

    let list = by_name(&requests, "listPets");
    assert_eq!(list.method, "GET");
    assert_eq!(list.url, "{{baseUrl}}/pets?limit={{limit}}");
    assert_eq!(path_of(&requests, "listPets"), "pets/listpets.http#0");
    assert_eq!(
        list.auth,
        Auth::None,
        "`security: []` on an operation means explicitly none"
    );
    let written = file_text(dir.path(), "petstore", "pets/listpets.http");
    assert!(written.contains("# Returns a page of pets."), "{written}");
    assert!(
        written.contains("#   status={{status}} — Filter by status"),
        "an optional parameter a .http file cannot disable is written where a reader will find it:\n{written}"
    );
    assert!(
        written.contains("#   X-Request-Id={{X-Request-Id}}"),
        "{written}"
    );

    let create = by_name(&requests, "createPet");
    assert_eq!(create.method, "POST");
    assert_eq!(
        create.auth,
        Auth::inherited(Auth::Bearer {
            token: "{{BearerAuth}}".to_string()
        })
    );
    assert_eq!(
        create.headers,
        vec![("Content-Type".to_string(), "application/json".to_string())]
    );
    let body: serde_json::Value =
        serde_json::from_str(create.body.as_text().unwrap()).expect("a JSON body");
    assert_eq!(
        body,
        serde_json::json!({
            "name": "Fido",
            "bornAt": "2026-01-01T00:00:00Z",
            "status": "available",
            "tags": [{"id": 0, "label": "string"}],
            "weightKg": 0.0,
            "microchipped": true
        })
    );

    let get = by_name(&requests, "getPet");
    assert_eq!(
        get.url, "{{baseUrl}}/pets/{{petId}}",
        "a path parameter becomes a variable the user can set"
    );

    let delete = by_name(&requests, "deletePet");
    assert_eq!(
        delete.headers,
        vec![("X-Api-Key".to_string(), "{{ApiKeyAuth}}".to_string())],
        "an operation's own security overrides the document's, and an api key is the header it always was"
    );
    assert_eq!(delete.auth, Auth::None);
    assert!(warning_with(&report, "deletePet").contains("X-Api-Key"));
    assert!(file_text(dir.path(), "petstore", "pets/deletepet.http").contains("# DEPRECATED"));

    let order = by_name(&requests, "placeOrder");
    assert_eq!(
        order.body,
        Body::Raw {
            language: RawLanguage::Text,
            text: "petId=string&quantity=1".to_string()
        }
    );
    assert_eq!(
        order.headers,
        vec![(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded".to_string()
        )]
    );

    assert_eq!(by_name(&requests, "readProfile").auth, Auth::None);
    let upload = by_name(&requests, "uploadPetPhoto");
    assert_eq!(
        upload.body,
        Body::Formdata {
            rows: vec![
                FormDataRow::text("caption", "string"),
                FormDataRow::text("file", ""),
            ]
        },
        "a `format: binary` property is a file field with no file to point at yet"
    );
    let upload_file = file_text(dir.path(), "petstore", "pets/uploadpetphoto.http");
    assert!(
        upload_file.contains("Content-Type: multipart/form-data")
            && upload_file.contains("caption = string")
            && upload_file.contains("# file = < ./your-file"),
        "the file field is written as a row and the comment says how to fill it: {upload_file}"
    );
    assert_eq!(
        warning_with(&report, "uploadPetPhoto"),
        "uploadPetPhoto: the form-data file fields (file) arrived empty — a spec names no file a workspace could hold, so point each at one with `file = < ./your-file`"
    );

    assert_eq!(report.skipped, Vec::<String>::new());
    assert!(warning_with(&report, "OAuth 2.0").contains("authorizationCode"));
    assert!(warning_with(&report, "tagged").contains("createPet"));
    assert!(warning_with(&report, "application/xml").contains("createPet"));
}

#[test]
fn every_server_becomes_its_own_environment_with_its_own_variables() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("petstore-3.0.json")).unwrap();

    let production = env_named(dir.path(), "Petstore-Production");
    assert_eq!(production["baseUrl"], "https://api.petstore.dev/v2");
    assert_eq!(production["petId"], "00000000-0000-0000-0000-000000000000");
    assert_eq!(production["limit"], "20");
    assert!(!production.contains_key("tenant"));

    let staging = env_named(dir.path(), "Petstore-Staging");
    assert_eq!(
        staging["baseUrl"], "https://{{tenant}}.staging.petstore.dev/v2",
        "a server variable stays a variable, seeded from its default"
    );
    assert_eq!(staging["tenant"], "acme");

    let docs = workspace::list_env_docs(dir.path()).unwrap();
    let production = docs
        .items
        .iter()
        .find(|d| d.name == "Petstore-Production")
        .unwrap();
    assert!(
        production.vars["BearerAuth"].is_secret(),
        "a token is declared, never invented"
    );
    assert!(production.vars["ApiKeyAuth"].is_secret());
    assert!(warning_with(&report, "mandalo env set").contains("BearerAuth"));
    assert!(warning_with(&report, "2 servers").contains("Petstore-Production"));
}

#[test]
fn a_3_1_yaml_spec_imports_with_type_arrays_and_reports_its_webhooks() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("inventory-3.1.yaml")).unwrap();
    assert_eq!(report.imported, 3);

    let requests = imported(dir.path());
    let replace = by_name(&requests, "replaceItem");
    assert_eq!(replace.url, "{{baseUrl}}/items/{{sku}}");
    let body: serde_json::Value = serde_json::from_str(replace.body.as_text().unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "sku": "SKU-1001",
            "name": "string",
            "discontinuedAt": "2026-01-01T00:00:00Z",
            "kind": "physical",
            "quantity": 0,
            "location": {"warehouse": "string", "aisle": 0}
        }),
        "3.1 writes nullable as a type array; the non-null arm is the useful one"
    );
    assert_eq!(
        replace.auth,
        Auth::inherited(Auth::Basic {
            username: "{{BasicAuthUser}}".to_string(),
            password: "{{BasicAuthPassword}}".to_string()
        })
    );

    let search = by_name(&requests, "searchItems");
    assert_eq!(search.url, "{{baseUrl}}/items/search?q={{q}}");
    assert!(file_text(dir.path(), "inventory", "items/searchitems.http")
        .contains("#   warehouse={{warehouse}} — Restrict to one warehouse"));

    assert!(warning_with(&report, "webhook").contains("1 webhook"));
}

#[test]
fn a_swagger_2_0_spec_converts_host_basepath_schemes_and_body_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("billing-swagger-2.0.json")).unwrap();
    assert_eq!(report.imported, 3);
    assert!(report.summary.contains("Swagger 2.0"));

    let env = env_named(dir.path(), "Billing-https-billing-example-com");
    assert_eq!(env["baseUrl"], "https://billing.example.com/v1");

    let requests = imported(dir.path());
    let list = by_name(&requests, "listInvoices");
    assert_eq!(
        list.url, "{{baseUrl}}/invoices?customerId={{customerId}}&api_key={{apiKeyQuery}}",
        "an api key in the query goes where it was always going: the URL"
    );
    assert_eq!(list.auth, Auth::None);

    let create = by_name(&requests, "createInvoice");
    assert_eq!(
        create.auth,
        Auth::inherited(Auth::Basic {
            username: "{{basicAuthUser}}".to_string(),
            password: "{{basicAuthPassword}}".to_string()
        })
    );
    let body: serde_json::Value = serde_json::from_str(create.body.as_text().unwrap()).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "customerId": "cus_1",
            "dueOn": "2026-01-01",
            "lines": [{"description": "string", "amountCents": 0}]
        })
    );

    let attach = by_name(&requests, "attachToInvoice");
    assert_eq!(
        attach.url, "{{baseUrl}}/invoices/{{invoiceId}}/attachments?api_key={{apiKeyQuery}}",
        "the document-wide requirement still applies to an operation that declares none"
    );
    assert_eq!(
        attach.body,
        Body::Formdata {
            rows: vec![
                FormDataRow::text("note", "string"),
                FormDataRow::text("receipt", ""),
            ]
        },
        "a Swagger `type: file` form field forces the multipart body it always needed"
    );
    assert_eq!(
        warning_with(&report, "attachToInvoice: the body"),
        "attachToInvoice: the body was written as multipart/form-data — the operation declares a file field, which no other body carries"
    );
    assert_eq!(
        warning_with(&report, "attachToInvoice: the form-data"),
        "attachToInvoice: the form-data file fields (receipt) arrived empty — a spec names no file a workspace could hold, so point each at one with `receipt = < ./your-file`"
    );
    assert_eq!(report.skipped, Vec::<String>::new());
}

#[test]
fn a_recursive_schema_terminates_and_says_where() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("tree-recursive-3.1.yaml")).unwrap();
    assert_eq!(report.imported, 2);

    let requests = imported(dir.path());
    let node: serde_json::Value =
        serde_json::from_str(by_name(&requests, "createNode").body.as_text().unwrap()).unwrap();
    assert_eq!(
        node,
        serde_json::json!({
            "id": "string",
            "label": "string",
            "parent": null,
            "children": [null]
        })
    );

    let graph: serde_json::Value =
        serde_json::from_str(by_name(&requests, "createGraph").body.as_text().unwrap()).unwrap();
    assert_eq!(
        graph,
        serde_json::json!({"name": "string", "right": {"code": 0, "left": null}}),
        "a mutually recursive pair terminates on the second lap, not the first"
    );

    assert!(warning_with(&report, "createNode").contains("contains itself"));
    assert!(warning_with(&report, "createGraph").contains("contains itself"));
}

#[test]
fn all_of_composes_and_one_of_picks_the_first_arm_out_loud() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("shapes-composition-3.0.json")).unwrap();
    let requests = imported(dir.path());

    let dog: serde_json::Value =
        serde_json::from_str(by_name(&requests, "createAnimal").body.as_text().unwrap()).unwrap();
    assert_eq!(
        dog,
        serde_json::json!({
            "name": "string",
            "ageYears": 0,
            "breed": "corgi",
            "goodBoy": true
        }),
        "every allOf arm contributes its properties"
    );

    let payment: serde_json::Value =
        serde_json::from_str(by_name(&requests, "createPayment").body.as_text().unwrap()).unwrap();
    assert_eq!(
        payment,
        serde_json::json!({"kind": "card", "last4": "4242"})
    );
    assert!(warning_with(&report, "oneOf").contains("first of 2"));
}

#[test]
fn a_remote_ref_is_refused_by_name_and_never_fetched() {
    let dir = tempfile::tempdir().unwrap();
    let err = openapi::import(dir.path(), &fixture("remote-ref-3.0.yaml")).unwrap_err();
    assert_eq!(err.code(), "E_UNSUPPORTED");
    let message = err.to_string();
    assert!(
        message.contains("https://schemas.example.com/pet.json#/Pet"),
        "{message}"
    );
    assert!(message.contains("./shared/owner.yaml#/Owner"), "{message}");
    assert!(message.contains("never fetches"), "{message}");
    assert!(
        collection::list_collections(dir.path())
            .unwrap()
            .items
            .is_empty(),
        "a refused import writes nothing at all"
    );
}

#[test]
fn a_relative_file_ref_is_refused_too() {
    let dir = tempfile::tempdir().unwrap();
    let source = serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Split", "version": "1"},
        "paths": {"/a": {"get": {"operationId": "a", "parameters": [
            {"$ref": "./params.yaml#/Limit"}
        ]}}}
    })
    .to_string();
    let err = openapi::import(dir.path(), &source)
        .unwrap_err()
        .to_string();
    assert!(err.contains("./params.yaml#/Limit"), "{err}");
}

#[test]
fn a_pathological_spec_is_bounded_instead_of_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let mut properties = serde_json::Map::new();
    for depth in 0..64 {
        properties.insert(
            format!("level{depth}"),
            serde_json::json!({"$ref": format!("#/components/schemas/S{}", depth + 1)}),
        );
    }
    let mut schemas = serde_json::Map::new();
    for depth in 0..64 {
        schemas.insert(
            format!("S{depth}"),
            serde_json::json!({"type": "object", "properties": properties}),
        );
    }
    schemas.insert("S64".to_string(), serde_json::json!({"type": "string"}));
    let source = serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Bomb", "version": "1"},
        "paths": {"/x": {"post": {"operationId": "explode", "requestBody": {"content": {
            "application/json": {"schema": {"$ref": "#/components/schemas/S0"}}
        }}}}},
        "components": {"schemas": schemas}
    })
    .to_string();

    let started = std::time::Instant::now();
    let report = openapi::import(dir.path(), &source).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the import has to be bounded, not merely finite"
    );
    assert_eq!(report.imported, 1);
    assert!(warning_with(&report, "explode").contains("skeleton"));
}

#[test]
fn more_operations_than_the_limit_stop_and_say_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = serde_json::Map::new();
    for index in 0..2_100 {
        paths.insert(
            format!("/p{index:04}"),
            serde_json::json!({"get": {"operationId": format!("op{index:04}")}}),
        );
    }
    let source = serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Wide", "version": "1"},
        "servers": [{"url": "https://wide.example.com"}],
        "paths": paths
    })
    .to_string();
    let report = openapi::import(dir.path(), &source).unwrap();
    assert_eq!(report.imported, 2_000);
    assert!(report.skipped[0].contains("more than 2000 operations"));
}

#[test]
fn importing_the_same_spec_twice_never_touches_the_first_collection() {
    let dir = tempfile::tempdir().unwrap();
    openapi::import(dir.path(), &fixture("petstore-3.0.json")).unwrap();

    let edited = collection::load_request_source(dir.path(), "petstore", "pets/getpet.http#0")
        .map(|mut request| {
            request.scripts.post = Some("pm.test('mine', () => {});".to_string());
            request
        })
        .unwrap();
    collection::put_request(dir.path(), "petstore", "pets/getpet.http#0", &edited).unwrap();

    let report = openapi::import(dir.path(), &fixture("petstore-3.0.json")).unwrap();
    assert!(report.summary.contains("petstore-2"));

    let slugs: Vec<String> = collection::list_collections(dir.path())
        .unwrap()
        .items
        .into_iter()
        .map(|c| c.slug)
        .collect();
    assert_eq!(slugs, vec!["petstore", "petstore-2"]);
    assert_eq!(
        collection::load_request_source(dir.path(), "petstore", "pets/getpet.http#0")
            .unwrap()
            .scripts
            .post
            .as_deref(),
        Some("pm.test('mine', () => {});"),
        "a re-import must not overwrite work the user did on the first one"
    );
}

#[test]
fn named_request_examples_become_one_block_each() {
    let dir = tempfile::tempdir().unwrap();
    let report = openapi::import(dir.path(), &fixture("examples-3.0.yaml")).unwrap();
    assert_eq!(
        report.imported, 5,
        "3 for createPet, 1 for getPet, 1 for createOwner"
    );

    let requests = imported(dir.path());
    for (label, expected) in [
        (
            "full",
            serde_json::json!({"name": "Fido", "breed": "corgi", "tags": ["good", "boy"]}),
        ),
        ("minimal", serde_json::json!({"name": "Fido"})),
        (
            "rescue",
            serde_json::json!({"name": "Luna", "rescuedOn": "2026-01-01"}),
        ),
    ] {
        let request = by_name(&requests, &format!("createPet ({label})"));
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "{{baseUrl}}/pets");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(request.body.as_text().unwrap()).unwrap(),
            expected,
            "{label}"
        );
    }

    let full = file_text(dir.path(), "examples", "pets/createpet-full.http");
    assert!(
        full.contains(r#"# Body: the spec's "full" example — Everything the API accepts"#),
        "{full}"
    );
    let rescue = file_text(dir.path(), "examples", "pets/createpet-rescue.http");
    assert!(
        rescue.contains(r#"# Body: the spec's "rescue" example."#),
        "an example with no summary still says which one it is:\n{rescue}"
    );
    assert!(warning_with(&report, "createPet").contains("names 3 request examples"));
}

#[test]
fn a_single_named_example_keeps_the_operation_name_it_always_had() {
    let dir = tempfile::tempdir().unwrap();
    openapi::import(dir.path(), &fixture("examples-3.0.yaml")).unwrap();
    let requests = imported(dir.path());

    let owner = by_name(&requests, "createOwner");
    assert_eq!(
        path_of(&requests, "createOwner"),
        "pets/createowner.http#0",
        "one example is not a fan-out, so nothing about the name changes"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(owner.body.as_text().unwrap()).unwrap(),
        serde_json::json!({"email": "ada@example.com"})
    );
    assert!(file_text(dir.path(), "examples", "pets/createowner.http")
        .contains(r#"# Body: the spec's "only" example — The single shape this endpoint takes"#));
}

#[test]
fn a_parameter_with_several_examples_seeds_one_and_offers_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    openapi::import(dir.path(), &fixture("examples-3.0.yaml")).unwrap();

    assert_eq!(
        env_named(dir.path(), "Examples-examples-example-com")["petId"],
        "pet_1",
        "an environment holds one value, so the first named example seeds it"
    );
    let written = file_text(dir.path(), "examples", "pets/getpet.http");
    assert!(
        written.contains(
            r#"#   petId=pet_404 — example "missing": A pet it does not, to see the 404"#
        ),
        "the alternatives have to be somewhere a reader can copy them:\n{written}"
    );
    assert!(
        !written.contains("pet_1 — example"),
        "the one that became the seed is not repeated as an alternative:\n{written}"
    );
}

#[test]
fn a_response_example_is_reference_prose_and_never_an_assertion() {
    let dir = tempfile::tempdir().unwrap();
    openapi::import(dir.path(), &fixture("examples-3.0.yaml")).unwrap();

    let written = file_text(dir.path(), "examples", "pets/getpet.http");
    assert!(
        written.contains(
            "# Example 200 response from the specification — reference only, nothing here asserts it:"
        ),
        "{written}"
    );
    assert!(written.contains(r#"#   "name": "Fido""#), "{written}");

    for (_, request) in imported(dir.path()) {
        assert!(
            request.tests.is_empty() && request.captures.is_empty(),
            "{}: an import must not write an assertion it invented",
            request.name
        );
        assert_eq!(
            request.scripts.post, None,
            "{}: nor a response script",
            request.name
        );
    }
}

#[test]
fn a_long_response_example_is_cut_instead_of_flooding_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let rows: Vec<serde_json::Value> = (0..200)
        .map(|n| serde_json::json!({"id": n, "name": format!("row {n}")}))
        .collect();
    let source = serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Long", "version": "1"},
        "servers": [{"url": "https://long.example.com"}],
        "paths": {"/rows": {"get": {"operationId": "listRows", "responses": {"200": {
            "description": "Rows",
            "content": {"application/json": {"example": rows}}
        }}}}}
    })
    .to_string();
    openapi::import(dir.path(), &source).unwrap();

    let written = file_text(dir.path(), "long", "listrows.http");
    assert!(written.contains("… cut here — the whole example is in the specification"));
    assert!(
        written.lines().count() < 25,
        "the comment block must not dwarf the request:\n{written}"
    );
}

#[test]
fn more_named_examples_than_the_limit_stop_and_name_what_was_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let examples: serde_json::Map<String, serde_json::Value> = (0..14)
        .map(|n| {
            (
                format!("case{n:02}"),
                serde_json::json!({"value": {"n": n}}),
            )
        })
        .collect();
    let source = serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Many", "version": "1"},
        "servers": [{"url": "https://many.example.com"}],
        "paths": {"/cases": {"post": {"operationId": "postCase", "requestBody": {"content": {
            "application/json": {"examples": examples}
        }}}}}
    })
    .to_string();
    let report = openapi::import(dir.path(), &source).unwrap();

    assert_eq!(report.imported, 10);
    assert_eq!(
        report.skipped,
        vec![
            "postCase: 4 of the 14 named request examples were not imported — an operation writes at most 10 requests (case10, case11, case12, case13)"
        ]
    );
}

fn runnable_spec(base: &str) -> String {
    serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Mock API", "version": "1"},
        "servers": [{"url": base, "description": "Local"}],
        "tags": [{"name": "core"}],
        "paths": {
            "/get": {
                "get": {
                    "operationId": "readThings",
                    "tags": ["core"],
                    "parameters": [
                        {
                            "name": "page",
                            "in": "query",
                            "required": true,
                            "schema": {"type": "integer", "default": 2}
                        },
                        {
                            "name": "X-Tenant",
                            "in": "header",
                            "required": true,
                            "schema": {"type": "string", "default": "acme"}
                        }
                    ],
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/post": {
                "post": {
                    "operationId": "createThing",
                    "tags": ["core"],
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string", "example": "nova"},
                                "count": {"type": "integer", "default": 3}
                            }
                        }}}
                    },
                    "responses": {"201": {"description": "created"}}
                }
            },
            "/echo": {
                "post": {
                    "operationId": "echoThing",
                    "tags": ["core"],
                    "requestBody": {
                        "content": {"application/json": {"examples": {
                            "small": {"summary": "A small one", "value": {"n": 1}},
                            "large": {"summary": "A large one", "value": {"n": 999}}
                        }}}
                    },
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/status/{code}": {
                "get": {
                    "operationId": "readStatus",
                    "tags": ["core"],
                    "parameters": [{
                        "name": "code",
                        "in": "path",
                        "required": true,
                        "schema": {"type": "integer", "example": 200}
                    }],
                    "responses": {"200": {"description": "ok"}}
                }
            }
        }
    })
    .to_string()
}

#[tokio::test]
async fn the_imported_collection_actually_runs_against_the_mock() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockApi::start().await;
    let report = openapi::import(dir.path(), &runnable_spec(&api.base_url())).unwrap();
    assert_eq!(
        report.imported, 5,
        "echoThing fans out into one request per example"
    );
    assert_eq!(report.environments, 1);

    let run = fixtures::runner()
        .run_suite(dir.path(), "mock-api", None, Some("Mock-API-Local"))
        .await
        .unwrap();
    assert_eq!(run.total, 5);
    assert!(
        run.passed,
        "the imported suite must run: {:?}",
        run.steps
            .iter()
            .flat_map(|s| s.failures())
            .collect::<Vec<_>>()
    );
    for step in &run.steps {
        assert_eq!(
            step.response.as_ref().map(|r| r.status),
            Some(200),
            "{}",
            step.request_name
        );
    }

    let read = &api.requests_to("/get")[0];
    assert_eq!(read.query.get("page").map(String::as_str), Some("2"));
    assert_eq!(read.header("x-tenant"), Some("acme"));

    let created = &api.requests_to("/post")[0];
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&created.body).unwrap(),
        serde_json::json!({"name": "nova", "count": 3})
    );
    assert_eq!(created.header("content-type"), Some("application/json"));

    assert_eq!(api.requests_to("/status/200").len(), 1);

    let echoed: Vec<serde_json::Value> = api
        .requests_to("/echo")
        .iter()
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect();
    assert_eq!(
        echoed,
        vec![serde_json::json!({"n": 999}), serde_json::json!({"n": 1})],
        "both named examples were sent, each as its own request"
    );
}

/// 3.1 spells a file as `contentMediaType` where 3.0 spells it `format: binary`,
/// and `encoding` names what a part is sent as.
#[test]
fn a_3_1_multipart_body_becomes_field_lines_and_names_the_file_it_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let source = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "Filing", "version": "1"},
        "paths": {"/filings": {"post": {
            "operationId": "fileReport",
            "requestBody": {"content": {"multipart/form-data": {
                "schema": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "example": "Q3 expenses"},
                        "quarter": {"type": "integer", "default": 3},
                        "report": {"type": "string", "contentMediaType": "application/pdf"}
                    }
                },
                "encoding": {"report": {"contentType": "application/x-invoice"}}
            }}},
            "responses": {"200": {"description": "ok"}}
        }}}
    })
    .to_string();
    let report = openapi::import(dir.path(), &source).unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(report.skipped, Vec::<String>::new());
    assert_eq!(
        warning_with(&report, "fileReport"),
        "fileReport: the form-data file fields (report) arrived empty — a spec names no file a workspace could hold, so point each at one with `report = < ./your-file`"
    );

    let requests = imported(dir.path());
    assert_eq!(
        by_name(&requests, "fileReport").body,
        Body::Formdata {
            rows: vec![
                FormDataRow::text("quarter", "3"),
                FormDataRow::text("report", ""),
                FormDataRow::text("title", "Q3 expenses"),
            ]
        }
    );
    let written = file_text(dir.path(), "filing", "filereport.http");
    assert!(
        written.contains(
            "# report = < ./your-file   (the spec declares it as a file, sent as application/x-invoice)"
        ),
        "`encoding` says what the part is sent as, and the comment keeps it: {written}"
    );
}

/// An all-text multipart body loses nothing, so it must import silently and
/// reach the server as the parts the schema described.
#[tokio::test]
async fn a_text_only_multipart_body_imports_clean_and_sends_its_parts() {
    let dir = tempfile::tempdir().unwrap();
    let api = MockApi::start().await;
    let source = serde_json::json!({
        "openapi": "3.0.3",
        "info": {"title": "Filing", "version": "1"},
        "servers": [{"url": api.base_url(), "description": "Local"}],
        "paths": {"/body/multipart": {"post": {
            "operationId": "fileReport",
            "requestBody": {"content": {"multipart/form-data": {"schema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string", "example": "Q3 expenses"},
                    "quarter": {"type": "integer", "default": 3}
                }
            }}}},
            "responses": {"200": {"description": "ok"}}
        }}}
    })
    .to_string();
    let report = openapi::import(dir.path(), &source).unwrap();
    assert_eq!(report.warnings, Vec::<String>::new());
    assert_eq!(report.skipped, Vec::<String>::new());

    let run = fixtures::runner()
        .run_suite(dir.path(), "filing", None, Some("Filing-Local"))
        .await
        .unwrap();
    assert!(run.passed, "{:?}", run.steps[0].failures());

    let echo: serde_json::Value =
        serde_json::from_str(&run.steps[0].response.as_ref().unwrap().body).unwrap();
    assert_eq!(echo["count"], 2);
    assert!(echo["contentType"]
        .as_str()
        .unwrap()
        .starts_with("multipart/form-data; boundary="));
    assert_eq!(echo["parts"][0]["name"], "quarter");
    assert_eq!(echo["parts"][0]["text"], "3");
    assert_eq!(echo["parts"][1]["name"], "title");
    assert_eq!(echo["parts"][1]["text"], "Q3 expenses");
}
