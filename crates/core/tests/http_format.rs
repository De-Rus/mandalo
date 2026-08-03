use mandalo_core::assertions::Scripts;
use mandalo_core::body::Body;
use mandalo_core::collection::SavedRequest;
use mandalo_core::http_format::{render_request, HttpDoc};
use mandalo_core::request::Auth;

const MULTI: &str = "\
### Login
# @name login
POST {{baseUrl}}/auth/login
Content-Type: application/json

{
  \"user\": \"ada\"
}

### Get profile
GET {{baseUrl}}/me
Authorization: Bearer {{token}}
";

fn doc(source: &str) -> HttpDoc {
    HttpDoc::parse("api", source).expect("the file parses")
}

fn request_at(source: &str, index: usize) -> SavedRequest {
    doc(source).raw(index).expect("the request exists").request
}

#[test]
fn a_file_holds_many_requests_separated_by_hashes() {
    let parsed = doc(MULTI);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed.names(), vec!["Login", "Get profile"]);

    let login = parsed.raw(0).unwrap().request;
    assert_eq!(login.method, "POST");
    assert_eq!(login.url, "{{baseUrl}}/auth/login");
    assert_eq!(
        login.headers,
        vec![("Content-Type".to_string(), "application/json".to_string())]
    );
    assert_eq!(login.body.as_text(), Some("{\n  \"user\": \"ada\"\n}"));

    let profile = parsed.raw(1).unwrap().request;
    assert_eq!(profile.method, "GET");
    assert_eq!(profile.headers, Vec::new());
    assert_eq!(
        profile.auth,
        Auth::Bearer {
            token: "{{token}}".to_string()
        }
    );
}

#[test]
fn the_separator_names_the_request_and_at_name_is_the_fallback() {
    let named = request_at("### Real name\n# @name ignored\nGET https://x.dev\n", 0);
    assert_eq!(named.name, "Real name");

    let metadata = request_at("###\n# @name from-metadata\nGET https://x.dev\n", 0);
    assert_eq!(metadata.name, "from-metadata");

    let derived = request_at("POST https://x.dev/users\n", 0);
    assert_eq!(derived.name, "POST https://x.dev/users");
}

#[test]
fn the_method_is_optional_and_defaults_to_get() {
    let request = request_at("https://x.dev/ping\n", 0);
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, "https://x.dev/ping");
}

#[test]
fn a_lowercase_method_and_a_trailing_http_version_are_accepted() {
    let request = request_at("post https://x.dev/a HTTP/1.1\n", 0);
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "https://x.dev/a");
}

#[test]
fn an_unknown_method_fails_loud_with_its_line() {
    let error = HttpDoc::parse("api", "### A\nFETCH https://x.dev\n").unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(error.to_string().contains("line 2"), "{error}");
    assert!(
        error
            .to_string()
            .contains("\"FETCH\" is not an HTTP method"),
        "{error}"
    );
}

#[test]
fn headers_carry_vars_in_names_and_values() {
    let request = request_at("GET https://x.dev\nX-{{region}}: {{token}}\n", 0);
    assert_eq!(
        request.headers,
        vec![("X-{{region}}".to_string(), "{{token}}".to_string())]
    );
}

#[test]
fn a_malformed_header_line_fails_loud_with_its_line() {
    let error = HttpDoc::parse("api", "GET https://x.dev\nnot a header\n").unwrap_err();
    assert!(error.to_string().contains("line 2"), "{error}");
    assert!(
        error.to_string().contains("expected `Name: value`"),
        "{error}"
    );
}

#[test]
fn a_body_keeps_its_blank_lines_and_indentation() {
    let source = "POST https://x.dev\nContent-Type: text/plain\n\nfirst\n\n  indented\n\nlast\n";
    let request = request_at(source, 0);
    assert_eq!(request.body.as_text(), Some("first\n\n  indented\n\nlast"));
}

#[test]
fn a_file_body_resolves_workspace_relative_and_rejects_traversal() {
    let request = request_at("POST https://x.dev\n\n< ./fixtures/payload.json\n", 0);
    assert_eq!(
        request.body,
        Body::Binary {
            file: "fixtures/payload.json".to_string(),
            content_type: None
        }
    );

    for bad in ["< ../secret.json", "< /etc/passwd", "< fixtures/../../x"] {
        let error = HttpDoc::parse("api", &format!("POST https://x.dev\n\n{bad}\n")).unwrap_err();
        assert_eq!(error.code(), "E_PATH_ESCAPE", "{bad}: {error}");
    }
}

#[test]
fn an_interpolated_file_body_is_rejected_rather_than_half_supported() {
    let error = HttpDoc::parse("api", "POST https://x.dev\n\n<@ ./payload.json\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("`<@`"), "{error}");
}

#[test]
fn file_variables_win_over_the_environment_because_they_are_more_local() {
    let source = "@host = file.dev\n\n### Ping\nGET https://{{host}}/{{path}}\n";
    let parsed = doc(source);
    assert_eq!(
        parsed.raw(0).unwrap().request.url,
        "https://{{host}}/{{path}}"
    );
    assert_eq!(
        parsed.resolved(0).unwrap().request.url,
        "https://file.dev/{{path}}"
    );
}

#[test]
fn a_file_variable_resolves_against_the_ones_declared_before_it() {
    let source = "@scheme = https\n@base = {{scheme}}://x.dev\n\nGET {{base}}/ping\n";
    assert_eq!(
        doc(source).resolved(0).unwrap().request.url,
        "https://x.dev/ping"
    );
}

#[test]
fn an_invalid_variable_name_fails_loud() {
    let error = HttpDoc::parse("api", "@a b = 1\nGET https://x.dev\n").unwrap_err();
    assert!(
        error.to_string().contains("is not a valid variable name"),
        "{error}"
    );
}

#[test]
fn script_blocks_map_onto_the_same_pm_engine() {
    let source = "\
### Login
< {%
pm.environment.set(\"nonce\", \"1\");
%}
POST https://x.dev/login

{\"a\":1}

> {%
pm.test(\"ok\", () => pm.response.to.have.status(200));
%}
";
    let request = request_at(source, 0);
    assert_eq!(
        request.scripts,
        Scripts {
            pre: Some("pm.environment.set(\"nonce\", \"1\");".to_string()),
            post: Some("pm.test(\"ok\", () => pm.response.to.have.status(200));".to_string()),
        }
    );
    assert_eq!(request.body.as_text(), Some("{\"a\":1}"));
}

#[test]
fn an_unclosed_script_block_fails_loud() {
    let error = HttpDoc::parse("api", "GET https://x.dev\n\n> {%\npm.test();\n").unwrap_err();
    assert!(error.to_string().contains("never closed"), "{error}");
}

#[test]
fn a_script_file_reference_is_rejected_rather_than_half_supported() {
    let error = HttpDoc::parse("api", "GET https://x.dev\n\n> ./after.js\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
}

#[test]
fn an_unknown_directive_fails_loud_instead_of_being_ignored() {
    let error = HttpDoc::parse("api", "# @no-redirect\nGET https://x.dev\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("@no-redirect"), "{error}");
}

#[test]
fn the_graphql_marker_selects_the_graphql_kind_and_splits_the_variables() {
    let source = "\
### User
POST https://x.dev/graphql
X-REQUEST-TYPE: GraphQL

query User($id: ID!) {
  user(id: $id) { name }
}

{\"id\": \"7\"}
";
    let request = request_at(source, 0);
    assert_eq!(request.kind, "graphql");
    assert!(
        request.headers.is_empty(),
        "the marker never reaches the wire"
    );
    assert_eq!(
        request.body,
        Body::Graphql {
            query: "query User($id: ID!) {\n  user(id: $id) { name }\n}".to_string(),
            variables: "{\"id\": \"7\"}".to_string(),
        }
    );
}

#[test]
fn a_long_url_continues_on_an_indented_line() {
    let request = request_at(
        "GET https://x.dev/search\n  ?q=hola\n  &page=2\nAccept: */*\n",
        0,
    );
    assert_eq!(request.url, "https://x.dev/search?q=hola&page=2");
    assert_eq!(request.headers.len(), 1);
}

#[test]
fn comments_are_skipped_outside_the_body_and_kept_inside_it() {
    let source = "# a note\n// another\nGET https://x.dev\n// between headers\nAccept: */*\n\n# inside the body\n";
    let request = request_at(source, 0);
    assert_eq!(request.headers.len(), 1);
    assert_eq!(request.body.as_text(), Some("# inside the body"));
}

#[test]
fn crlf_files_parse_to_the_same_requests_as_lf_files() {
    let crlf = MULTI.replace('\n', "\r\n");
    let parsed = doc(&crlf);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed.raw(0).unwrap().request.url, "{{baseUrl}}/auth/login");
    assert_eq!(
        parsed.raw(0).unwrap().request.body.as_text(),
        Some("{\r\n  \"user\": \"ada\"\r\n}")
    );
}

#[test]
fn a_file_without_a_trailing_newline_parses() {
    let parsed = doc("### Ping\nGET https://x.dev/ping");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed.raw(0).unwrap().request.url, "https://x.dev/ping");
}

#[test]
fn parsing_and_reserialising_an_untouched_file_is_byte_identical() {
    let crlf = MULTI.replace('\n', "\r\n");
    let no_newline = "### Ping\nGET https://x.dev/ping";
    let gnarly = "\
@host  =  x.dev

# a leading comment

###  Spaced   name
GET   https://{{host}}/a   HTTP/1.1
Accept:   */*


body line

> {%
pm.test(\"x\", () => {});
%}

### Second
POST https://{{host}}/b
";
    for source in [MULTI, crlf.as_str(), no_newline, gnarly] {
        let parsed = doc(source);
        assert_eq!(parsed.source(), source);
        for index in 0..parsed.len() {
            let request = parsed.raw(index).unwrap().request;
            assert_eq!(
                parsed.replace(index, &request).unwrap(),
                source,
                "re-saving request {index} unchanged rewrote the file"
            );
        }
    }
}

#[test]
fn a_url_only_edit_touches_one_line() {
    let parsed = doc(MULTI);
    let mut request = parsed.raw(1).unwrap().request;
    request.url = "{{baseUrl}}/profile".to_string();
    let out = parsed.replace(1, &request).unwrap();
    assert_eq!(out, MULTI.replace("{{baseUrl}}/me", "{{baseUrl}}/profile"));
}

#[test]
fn an_edit_keeps_comments_blank_lines_and_alignment() {
    let source = "\
### Login
# the service issues a short-lived token here
POST https://x.dev/login
Content-Type:    application/json
Accept: */*

{\"user\": \"ada\"}
";
    let parsed = doc(source);
    let mut request = parsed.raw(0).unwrap().request;
    request.headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/xml".to_string()),
    ];
    let out = parsed.replace(0, &request).unwrap();
    assert!(out.contains("# the service issues a short-lived token here"));
    assert!(out.contains("Content-Type:    application/json"));
    assert!(out.contains("Accept: application/xml"));
}

#[test]
fn adding_and_removing_a_header_edits_only_that_line() {
    let source = "GET https://x.dev\nAccept: */*\n\nbody\n";
    let parsed = doc(source);
    let mut request = parsed.raw(0).unwrap().request;
    request
        .headers
        .push(("X-Trace".to_string(), "t-1".to_string()));
    let added = parsed.replace(0, &request).unwrap();
    assert_eq!(
        added,
        "GET https://x.dev\nAccept: */*\nX-Trace: t-1\n\nbody\n"
    );

    let reparsed = doc(&added);
    let mut request = reparsed.raw(0).unwrap().request;
    request.headers.retain(|(k, _)| k != "X-Trace");
    assert_eq!(reparsed.replace(0, &request).unwrap(), source);
}

#[test]
fn a_body_can_be_added_and_removed() {
    let source = "POST https://x.dev\nAccept: */*\n";
    let parsed = doc(source);
    let mut request = parsed.raw(0).unwrap().request;
    request.body = Body::json("{\"a\":1}");
    let added = parsed.replace(0, &request).unwrap();
    assert_eq!(added, "POST https://x.dev\nAccept: */*\n\n{\"a\":1}\n");

    let reparsed = doc(&added);
    let mut request = reparsed.raw(0).unwrap().request;
    request.body = Body::None;
    assert_eq!(reparsed.replace(0, &request).unwrap(), source);
}

#[test]
fn rendering_a_request_and_parsing_it_back_returns_the_same_request() {
    let shapes = vec![
        SavedRequest {
            id: "api-0".to_string(),
            name: "Ping".to_string(),
            kind: "http".to_string(),
            method: "GET".to_string(),
            url: "{{baseUrl}}/ping".to_string(),
            description: None,
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            auth: Auth::None,
            body: Body::None,
            grpc: None,
            scripts: Scripts::default(),
            tests: Vec::new(),
            captures: Vec::new(),
        },
        SavedRequest {
            id: "api-0".to_string(),
            name: "Login".to_string(),
            kind: "http".to_string(),
            method: "POST".to_string(),
            url: "{{baseUrl}}/login".to_string(),
            description: None,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            auth: Auth::Bearer {
                token: "{{token}}".to_string(),
            },
            body: Body::json("{\"user\":\"ada\"}"),
            grpc: None,
            scripts: Scripts {
                pre: Some("pm.environment.set(\"n\", 1);".to_string()),
                post: Some("pm.environment.set(\"token\", pm.response.json().token);".to_string()),
            },
            tests: Vec::new(),
            captures: Vec::new(),
        },
        SavedRequest {
            id: "api-0".to_string(),
            name: "Basic".to_string(),
            kind: "http".to_string(),
            method: "GET".to_string(),
            url: "https://x.dev/private".to_string(),
            description: None,
            headers: Vec::new(),
            auth: Auth::Basic {
                username: "{{user}}".to_string(),
                password: "{{pass}}".to_string(),
            },
            body: Body::None,
            grpc: None,
            scripts: Scripts::default(),
            tests: Vec::new(),
            captures: Vec::new(),
        },
        SavedRequest {
            id: "api-0".to_string(),
            name: "Gql".to_string(),
            kind: "graphql".to_string(),
            method: "POST".to_string(),
            url: "{{baseUrl}}/graphql".to_string(),
            description: None,
            headers: Vec::new(),
            auth: Auth::None,
            body: Body::graphql("{ ping }", "{\"a\":1}"),
            grpc: None,
            scripts: Scripts::default(),
            tests: Vec::new(),
            captures: Vec::new(),
        },
    ];
    for shape in shapes {
        let text = render_request(&shape, "\n").expect("renders");
        let back = HttpDoc::parse("api", &text)
            .unwrap_or_else(|e| panic!("{} does not parse back: {e}\n{text}", shape.name))
            .raw(0)
            .unwrap()
            .request;
        assert_eq!(back, shape, "{text}");
    }
}

#[test]
fn a_request_the_format_cannot_express_fails_loud_on_the_way_out() {
    let base = SavedRequest {
        id: "api-0".to_string(),
        name: "Upload".to_string(),
        kind: "http".to_string(),
        method: "POST".to_string(),
        url: "https://x.dev/upload".to_string(),
        description: None,
        headers: Vec::new(),
        auth: Auth::None,
        body: Body::Formdata { rows: Vec::new() },
        grpc: None,
        scripts: Scripts::default(),
        tests: Vec::new(),
        captures: Vec::new(),
    };
    let error = render_request(&base, "\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("multipart"), "{error}");
}

#[test]
fn a_request_is_addressed_by_index_and_by_name() {
    let parsed = doc(MULTI);
    assert_eq!(parsed.index_of("0").unwrap(), 0);
    assert_eq!(parsed.index_of("Get profile").unwrap(), 1);
    assert_eq!(parsed.index_of("7").unwrap_err().code(), "E_NOT_FOUND");
    assert_eq!(parsed.index_of("Ghost").unwrap_err().code(), "E_NOT_FOUND");
}

#[test]
fn a_duplicate_name_must_be_addressed_by_index() {
    let parsed = doc("### Same\nGET https://x.dev/a\n\n### Same\nGET https://x.dev/b\n");
    assert_eq!(parsed.index_of("Same").unwrap_err().code(), "E_CONFLICT");
    assert_eq!(parsed.index_of("1").unwrap(), 1);
}

#[test]
fn a_request_can_be_appended_and_removed() {
    let parsed = doc(MULTI);
    let mut fresh = parsed.raw(1).unwrap().request;
    fresh.name = "Logout".to_string();
    fresh.url = "{{baseUrl}}/logout".to_string();
    fresh.auth = Auth::None;
    let grown = parsed.append(&fresh).unwrap();
    let regrown = doc(&grown);
    assert_eq!(regrown.len(), 3);
    assert_eq!(regrown.names()[2], "Logout");
    assert_eq!(regrown.remove(2).unwrap().trim_end(), MULTI.trim_end());
}

#[test]
fn a_block_without_a_request_line_fails_loud() {
    let error = HttpDoc::parse("api", "### Empty\n# only a comment\n").unwrap_err();
    assert!(error.to_string().contains("no request line"), "{error}");
}

#[test]
fn a_description_becomes_a_comment_on_a_new_block_and_cannot_be_edited_as_a_field() {
    let mut fresh = doc(MULTI).raw(0).unwrap().request;
    fresh.description = Some("issues a short-lived token".to_string());
    let text = render_request(&fresh, "\n").expect("a new block carries it as a comment");
    assert!(
        text.starts_with("### Login\n# issues a short-lived token\n"),
        "{text}"
    );
    assert_eq!(
        HttpDoc::parse("api", &text)
            .unwrap()
            .raw(0)
            .unwrap()
            .request
            .description,
        None,
        "a comment is prose, not a field the parser reports back"
    );

    let parsed = doc(MULTI);
    let error = parsed.replace(0, &fresh).unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(
        error.to_string().contains("change the comment in the file"),
        "{error}"
    );
}

#[test]
fn a_description_cannot_forge_a_separator_and_split_the_file() {
    let mut fresh = doc(MULTI).raw(0).unwrap().request;
    fresh.description = Some("### Injected\nGET https://evil.dev".to_string());
    let text = render_request(&fresh, "\n").expect("renders");
    let parsed = HttpDoc::parse("api", &text).expect("parses");
    assert_eq!(
        parsed.len(),
        1,
        "the description opened a second block: {text}"
    );
    assert_eq!(parsed.raw(0).unwrap().request.url, "{{baseUrl}}/auth/login");
}
