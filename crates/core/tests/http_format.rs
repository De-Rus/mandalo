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
            stream: None,
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
            stream: None,
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
            stream: None,
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
            stream: None,
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
        body: Body::Urlencoded { rows: Vec::new() },
        grpc: None,
        stream: None,
        scripts: Scripts::default(),
        tests: Vec::new(),
        captures: Vec::new(),
    };
    let error = render_request(&base, "\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("urlencoded"), "{error}");
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

#[test]
fn an_xml_body_is_a_body_not_a_file_reference() {
    for xml in [
        "<user/>",
        "<?xml version=\"1.0\"?>",
        "<user>\n  <name>ada</name>\n</user>",
    ] {
        let source = format!("POST https://x.dev\nContent-Type: application/xml\n\n{xml}\n");
        let request = request_at(&source, 0);
        assert_eq!(
            request.body.as_text(),
            Some(xml),
            "{xml:?} was not kept as a body"
        );
    }
}

#[test]
fn a_file_reference_needs_whitespace_after_the_angle_bracket() {
    let with_space = request_at("POST https://x.dev\n\n< ./payload.json\n", 0);
    assert_eq!(
        with_space.body,
        Body::Binary {
            file: "payload.json".to_string(),
            content_type: None
        }
    );
    let without_space = request_at("POST https://x.dev\n\n<./payload.json\n", 0);
    assert_eq!(without_space.body.as_text(), Some("<./payload.json"));
}

#[test]
fn an_xml_body_survives_a_round_trip_through_the_editor() {
    let source = "POST https://x.dev\nContent-Type: application/xml\n\n<user/>\n";
    let parsed = doc(source);
    let request = parsed.raw(0).unwrap().request;
    assert_eq!(parsed.replace(0, &request).unwrap(), source);
    let rendered = render_request(&request, "\n").unwrap();
    assert_eq!(
        HttpDoc::parse("api", &rendered)
            .unwrap()
            .raw(0)
            .unwrap()
            .request
            .body,
        request.body
    );
}

#[test]
fn a_request_line_with_a_method_and_nothing_else_names_the_missing_url() {
    for source in ["GET\n", "### One\npost\n"] {
        let error = HttpDoc::parse("api", source).unwrap_err();
        assert_eq!(error.code(), "E_PARSE", "{source:?}: {error}");
        assert!(
            error.to_string().contains("this request line has no URL"),
            "{source:?}: {error}"
        );
    }
}

#[test]
fn a_file_body_with_no_path_asks_for_one_instead_of_sending_a_bare_arrow() {
    let error = HttpDoc::parse("api", "POST https://x.dev\n\n<\n").unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(
        error.to_string().contains("the body file needs a path"),
        "{error}"
    );
}

#[test]
fn an_inherited_authorization_line_says_so_and_survives_a_roundtrip() {
    let source = "\
### Profile
# @auth inherited
POST {{baseUrl}}/me
Authorization: Bearer {{authToken}}
";
    let request = request_at(source, 0);
    assert_eq!(
        request.auth,
        Auth::inherited(Auth::Bearer {
            token: "{{authToken}}".to_string()
        })
    );
    assert!(request.headers.is_empty());
    assert_eq!(render_request(&request, "\n"), Ok(source.to_string()));

    let own = request_at(
        "### Me\nGET https://x.dev/me\nAuthorization: Bearer {{t}}\n",
        0,
    );
    assert_eq!(
        own.auth,
        Auth::Bearer {
            token: "{{t}}".to_string()
        },
        "a header the request wrote itself is not a default"
    );
}

#[test]
fn dropping_the_inherited_marker_rewrites_only_that_line() {
    let source = "\
### Profile
# @auth inherited
GET https://x.dev/me
Authorization: Bearer {{authToken}}
";
    let parsed = doc(source);
    let mut request = parsed.raw(0).unwrap().request;
    request.auth = Auth::Bearer {
        token: "{{authToken}}".to_string(),
    };
    assert_eq!(
        parsed.replace(0, &request).unwrap(),
        "### Profile\nGET https://x.dev/me\nAuthorization: Bearer {{authToken}}\n"
    );
}

#[test]
fn an_auth_directive_that_says_anything_else_fails_loud() {
    let error = HttpDoc::parse("api", "# @auth bearer\nGET https://x.dev\n").unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(
        error.to_string().contains("only takes `inherited`"),
        "{error}"
    );
}

#[test]
fn a_body_error_names_the_line_the_body_starts_on() {
    let error = HttpDoc::parse(
        "api",
        "### one\nGET https://x.dev/a\nContent-Type: text/plain\n\nbody one\n\n### two\nGET https://x.dev/b\n\n<@ ./payload.json\n",
    )
    .unwrap_err();
    assert!(error.to_string().starts_with("line 10:"), "{error}");

    let error = HttpDoc::parse(
        "api",
        "### one\nGET https://x.dev/a\n\n{\"a\": 1}\n\n### two\nGET https://x.dev/b\n\n< ./payload.json\ntrailing\n",
    )
    .unwrap_err();
    assert!(error.to_string().starts_with("line 9:"), "{error}");
}

#[test]
fn a_header_error_names_the_line_the_header_is_on() {
    let error = HttpDoc::parse(
        "api",
        "### one\nGET https://x.dev/a\n\nbody one\n\n### two\nGET https://x.dev/b\nX-REQUEST-TYPE: nope\n",
    )
    .unwrap_err();
    assert!(error.to_string().starts_with("line 8:"), "{error}");
}

#[test]
fn a_file_of_many_blocks_still_parses() {
    let mut source = String::new();
    for index in 0..4000 {
        source.push_str(&format!(
            "### block {index}\nPOST https://x.dev/{index}\nContent-Type: application/json\n\n{{\"n\": {index}}}\n\n"
        ));
    }
    let parsed = doc(&source);
    assert_eq!(parsed.len(), 4000);
    assert_eq!(parsed.raw(3999).unwrap().name, "block 3999");
}

#[test]
fn a_header_whose_value_is_only_whitespace_parses_as_empty() {
    for source in [
        "GET https://x.dev\nX-Trace: \n",
        "GET https://x.dev\nX-Trace:\t\n",
        "GET https://x.dev\r\nX-Trace: \r\n",
        "GET https://x.dev\r\nX-Trace:\t \r\n",
        "GET https://x.dev\nX-Trace:\n",
    ] {
        let request = request_at(source, 0);
        assert_eq!(
            request.headers,
            vec![("X-Trace".to_string(), String::new())],
            "{source:?}"
        );
    }
}

#[test]
fn a_basic_username_with_a_colon_is_refused_where_it_can_arrive() {
    let mut request = request_at("POST https://x.dev/a\n", 0);
    request.auth = Auth::Basic {
        username: "ada:b".to_string(),
        password: "lovelace".to_string(),
    };
    let error = render_request(&request, "\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(
        error.to_string().contains("cannot contain a colon"),
        "{error}"
    );
    assert_eq!(
        doc("POST https://x.dev/a\n")
            .replace(0, &request)
            .unwrap_err()
            .code(),
        "E_UNSUPPORTED",
        "saving one into an existing file is refused too"
    );
}

const MULTIPART: &str = "\
### Upload avatar
POST {{baseUrl}}/body/multipart
Content-Type: multipart/form-data; boundary=WebAppBoundary

--WebAppBoundary
Content-Disposition: form-data; name=\"title\"

Avatar shot
--WebAppBoundary
Content-Disposition: form-data; name=\"photo\"; filename=\"a.png\"
Content-Type: image/png

< files/a.png
--WebAppBoundary--
";

#[test]
fn a_multipart_body_parses_into_form_rows() {
    let request = request_at(MULTIPART, 0);
    let Body::Formdata { rows } = &request.body else {
        panic!("expected a formdata body, got {:?}", request.body);
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].key, "title");
    assert_eq!(rows[0].value, "Avatar shot");
    assert!(!rows[0].is_file());
    assert_eq!(rows[1].key, "photo");
    assert_eq!(rows[1].files, vec!["files/a.png"]);
    assert_eq!(rows[1].content_type.as_deref(), Some("image/png"));
    assert!(
        !request
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type")),
        "the multipart content type folds into the body model"
    );
}

#[test]
fn a_rendered_formdata_request_parses_back_to_the_same_rows() {
    let original = request_at(MULTIPART, 0);
    let rendered = render_request(&original, "\n").unwrap();
    let reread = request_at(&rendered, 0);
    assert_eq!(reread.body, original.body);
    assert_eq!(reread.method, original.method);
}

const FORM_FIELDS: &str = "\
### Upload avatar
POST {{baseUrl}}/body/multipart
Content-Type: multipart/form-data

title = Avatar shot
photo = < files/a.png; type=image/png
";

#[test]
fn a_field_per_line_form_body_parses_into_the_same_rows_as_the_boundary_form() {
    let readable = request_at(FORM_FIELDS, 0);
    let literal = request_at(MULTIPART, 0);
    assert_eq!(readable.body, literal.body);
    let Body::Formdata { rows } = &readable.body else {
        panic!("expected a formdata body, got {:?}", readable.body);
    };
    assert_eq!(rows[0].key, "title");
    assert_eq!(rows[0].value, "Avatar shot");
    assert_eq!(rows[1].files, vec!["files/a.png"]);
    assert_eq!(rows[1].content_type.as_deref(), Some("image/png"));
    assert!(
        !readable
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type")),
        "the multipart content type folds into the body model"
    );
}

#[test]
fn a_field_per_line_form_body_round_trips_byte_for_byte() {
    let rendered = render_request(&request_at(FORM_FIELDS, 0), "\n").unwrap();
    assert_eq!(rendered, FORM_FIELDS);
    assert_eq!(
        render_request(&request_at(&rendered, 0), "\n").unwrap(),
        rendered
    );
}

#[test]
fn one_field_named_three_times_is_three_files_in_order() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nattachments = < files/a.txt\nattachments = < files/b.txt\nattachments = < files/c.txt\n";
    let request = request_at(source, 0);
    let Body::Formdata { rows } = &request.body else {
        panic!("expected formdata");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key, "attachments");
    assert_eq!(
        rows[0].files,
        vec![
            "files/a.txt".to_string(),
            "files/b.txt".to_string(),
            "files/c.txt".to_string()
        ]
    );
    assert_eq!(
        render_request(&request, "\n").unwrap(),
        "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nattachments = < files/a.txt < files/b.txt < files/c.txt\n"
    );
}

#[test]
fn several_files_on_one_field_line_parse_and_round_trip() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nattachments = < ./files/a.txt < ./files/b.txt <./files/c.txt; type=text/plain\n";
    let request = request_at(source, 0);
    let Body::Formdata { rows } = &request.body else {
        panic!("expected formdata");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].files,
        vec![
            "files/a.txt".to_string(),
            "files/b.txt".to_string(),
            "files/c.txt".to_string()
        ]
    );
    assert_eq!(rows[0].content_type.as_deref(), Some("text/plain"));
    assert_eq!(
        render_request(&request, "\n").unwrap(),
        "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nattachments = < files/a.txt < files/b.txt < files/c.txt; type=text/plain\n"
    );
}

#[test]
fn the_older_name_angle_path_spelling_still_reads_and_is_rewritten() {
    let legacy = FORM_FIELDS.replace("photo = < files", "photo < files");
    let request = request_at(&legacy, 0);
    assert_eq!(request.body, request_at(FORM_FIELDS, 0).body);
    assert_eq!(render_request(&request, "\n").unwrap(), FORM_FIELDS);
}

#[test]
fn whitespace_around_the_separators_is_forgiving() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\ntitle=Avatar shot\nphoto=<./files/a.png;type=image/png\nlogo   =   <   files/b.png\n";
    let Body::Formdata { rows } = &request_at(source, 0).body else {
        panic!("expected formdata");
    };
    assert_eq!(rows[0].value, "Avatar shot");
    assert_eq!(rows[1].files, vec!["files/a.png"]);
    assert_eq!(rows[1].content_type.as_deref(), Some("image/png"));
    assert_eq!(rows[2].files, vec!["files/b.png"]);
}

#[test]
fn a_value_that_only_looks_like_markup_stays_a_text_field() {
    let source =
        "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nbio = <b>bold</b>\n";
    let Body::Formdata { rows } = &request_at(source, 0).body else {
        panic!("expected formdata");
    };
    assert!(!rows[0].is_file());
    assert_eq!(rows[0].value, "<b>bold</b>");
}

#[test]
fn a_text_value_that_would_read_back_as_a_file_is_refused() {
    let mut request = request_at(FORM_FIELDS, 0);
    request.body = Body::Formdata {
        rows: vec![mandalo_core::body::FormDataRow::text(
            "note",
            "< ./files/a.png",
        )],
    };
    let error = render_request(&request, "\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("file reference"), "{error}");
}

#[test]
fn file_scoped_vars_reach_a_field_per_line_path() {
    let source = "@dir = files\n### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nphoto = < {{dir}}/a.png\n";
    let request = doc(source).resolved(0).unwrap().request;
    let Body::Formdata { rows } = &request.body else {
        panic!("expected formdata");
    };
    assert_eq!(rows[0].files, vec!["files/a.png"]);
}

#[test]
fn a_field_per_line_path_may_not_leave_the_workspace() {
    for path in ["/etc/passwd", "../../secret.env", "C:/keys.pem"] {
        let source = format!(
            "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nleak = < {path}\n"
        );
        assert_eq!(
            HttpDoc::parse("api", &source).unwrap_err().code(),
            "E_PATH_ESCAPE",
            "{path}"
        );
    }
}

#[test]
fn a_form_field_without_a_separator_says_what_a_field_looks_like() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\njust words\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(
        error.contains("a form field reads `name = value`, or `name = < ./path`"),
        "{error}"
    );
}

#[test]
fn a_form_field_with_no_name_says_so() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\n = orphan\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("no name"), "{error}");
}

#[test]
fn only_type_is_a_form_file_parameter() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\nphoto = < files/a.png; charset=utf-8\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("`; type=…`"), "{error}");
}

#[test]
fn a_field_per_line_body_refuses_a_boundary_parameter() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\ntitle = hola\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(
        error.contains("remove the `boundary=` parameter"),
        "{error}"
    );
}

#[test]
fn a_form_value_the_readable_form_cannot_write_is_refused_rather_than_flattened() {
    let mut request = request_at(FORM_FIELDS, 0);
    request.body = Body::Formdata {
        rows: vec![mandalo_core::body::FormDataRow::text("notes", "one\ntwo")],
    };
    let error = render_request(&request, "\n").unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("more than one line"), "{error}");
}

#[test]
fn editing_a_boundary_form_block_keeps_the_boundary_form() {
    let parsed = doc(MULTIPART);
    let mut request = parsed.raw(0).unwrap().request;
    let Body::Formdata { rows } = &mut request.body else {
        panic!("expected formdata");
    };
    rows[0].value = "A new caption".to_string();
    let saved = parsed.replace(0, &request).unwrap();
    assert_eq!(
        saved,
        MULTIPART.replace("Avatar shot", "A new caption"),
        "only the value changed"
    );
}

#[test]
fn editing_a_field_per_line_block_touches_only_the_body() {
    let source = format!("{FORM_FIELDS}\n> {{%\npm.test(\"ok\", function () {{}});\n%}}\n");
    let parsed = doc(&source);
    let mut request = parsed.raw(0).unwrap().request;
    let Body::Formdata { rows } = &mut request.body else {
        panic!("expected formdata");
    };
    rows[0].value = "A new caption".to_string();
    assert_eq!(
        parsed.replace(0, &request).unwrap(),
        source.replace("Avatar shot", "A new caption")
    );
}

#[test]
fn multipart_without_a_boundary_parameter_is_refused() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data\n\n--x\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("boundary"), "{error}");
}

#[test]
fn an_unclosed_multipart_body_is_refused() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name=\"a\"\n\n1\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("never closed"), "{error}");
}

#[test]
fn inline_file_content_in_a_part_is_refused() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name=\"f\"; filename=\"a.png\"\n\nPNGBYTES\n--B--\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("< path"), "{error}");
}

#[test]
fn a_text_part_with_a_content_type_is_refused() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name=\"meta\"\nContent-Type: application/json\n\n{}\n--B--\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("text part"), "{error}");
}

#[test]
fn file_scoped_vars_reach_formdata_rows() {
    let source = "@dir = files\n### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name=\"photo\"; filename=\"a.png\"\n\n< {{dir}}/a.png\n--B--\n";
    let request = doc(source).resolved(0).unwrap().request;
    let Body::Formdata { rows } = &request.body else {
        panic!("expected formdata");
    };
    assert_eq!(rows[0].files, vec!["files/a.png"]);
}

#[test]
fn a_part_missing_its_blank_line_says_so() {
    let source = "### U\nPOST https://a.dev/x\nContent-Type: multipart/form-data; boundary=B\n\n--B\nContent-Disposition: form-data; name=\"a\"\n--B--\n";
    let error = HttpDoc::parse("api", source).unwrap_err().to_string();
    assert!(error.contains("blank line"), "{error}");
}

const FEED: &str = "\
### Prices
GET {{baseUrl}}/prices/stream
Accept: text/event-stream
Last-Event-ID: 42
";

#[test]
fn an_accept_header_is_what_makes_a_request_a_stream() {
    let doc = HttpDoc::parse("feed.http", FEED).expect("the file parses");
    let request = doc.raw(0).expect("the request").request;
    assert_eq!(request.kind, "sse");
    assert_eq!(request.method, "GET");
    // Accept is the marker and stays a header; the resume id is a field.
    assert_eq!(
        request.headers,
        vec![("Accept".to_string(), "text/event-stream".to_string())]
    );
    assert_eq!(
        request.stream.as_ref().unwrap().last_event_id.as_deref(),
        Some("42")
    );
    let stream = request.stream.as_ref().expect("a stream");
    assert_eq!(stream.auto_reconnect, None);
    assert_eq!(doc.replace(0, &request).expect("a save"), FEED);
}

#[test]
fn an_accept_header_with_parameters_and_alternatives_still_marks_a_stream() {
    for accept in [
        "text/event-stream",
        "TEXT/EVENT-STREAM",
        "text/event-stream;charset=utf-8",
        "application/json, text/event-stream;q=0.9",
    ] {
        let source = format!("### Feed\nGET https://x.dev/e\nAccept: {accept}\n");
        let doc = HttpDoc::parse("f.http", &source).expect("the file parses");
        assert_eq!(doc.raw(0).unwrap().request.kind, "sse", "{accept}");
    }
    let doc = HttpDoc::parse(
        "f.http",
        "### Plain\nGET https://x.dev/e\nAccept: application/json\n",
    )
    .expect("the file parses");
    assert_eq!(doc.raw(0).unwrap().request.kind, "http");
}

#[test]
fn reconnect_off_round_trips_and_appears_only_on_a_stream() {
    let source = "### Feed\n# @reconnect off\nGET https://x.dev/e\nAccept: text/event-stream\n";
    let doc = HttpDoc::parse("f.http", source).expect("the file parses");
    let mut request = doc.raw(0).expect("the request").request;
    assert_eq!(request.stream.as_ref().unwrap().auto_reconnect, Some(false));
    assert_eq!(doc.replace(0, &request).expect("a save"), source);

    request.stream.as_mut().unwrap().auto_reconnect = Some(true);
    assert_eq!(
        doc.replace(0, &request).expect("a save"),
        "### Feed\n# @reconnect on\nGET https://x.dev/e\nAccept: text/event-stream\n"
    );

    let err = HttpDoc::parse(
        "f.http",
        "### Plain\n# @reconnect off\nGET https://x.dev/e\n",
    )
    .expect_err("a plain request has nothing to reconnect");
    assert!(
        err.to_string().contains("only means something to a stream"),
        "{err}"
    );
}

#[test]
fn a_stream_refuses_a_response_script_and_a_body_bearing_method() {
    let err = HttpDoc::parse(
        "f.http",
        "### Feed\nGET https://x.dev/e\nAccept: text/event-stream\n\n> {%\npm.test('x', function () {});\n%}\n",
    )
    .expect_err("a stream has no single response");
    assert!(err.to_string().contains("no single response"), "{err}");

    let err = HttpDoc::parse(
        "f.http",
        "### Feed\nPOST https://x.dev/e\nAccept: text/event-stream\n",
    )
    .expect_err("server-sent events arrive over a GET");
    assert!(err.to_string().contains("over a GET"), "{err}");
}

#[test]
fn a_stream_kind_and_its_accept_header_may_not_disagree() {
    let doc = HttpDoc::parse("feed.http", FEED).expect("the file parses");
    let mut request = doc.raw(0).expect("the request").request;
    request
        .headers
        .retain(|(k, _)| !k.eq_ignore_ascii_case("accept"));
    let err = doc.replace(0, &request).expect_err("the marker is gone");
    assert!(
        err.to_string().contains("Accept: text/event-stream"),
        "{err}"
    );

    let mut request = doc.raw(0).expect("the request").request;
    request.kind = "http".to_string();
    request.stream = None;
    let err = doc
        .replace(0, &request)
        .expect_err("the header still says stream");
    assert!(err.to_string().contains("reads back as a stream"), "{err}");
}

#[test]
fn a_rendered_stream_reads_back_as_the_same_stream() {
    let doc = HttpDoc::parse("feed.http", FEED).expect("the file parses");
    let mut request = doc.raw(0).expect("the request").request;
    request.stream.as_mut().unwrap().auto_reconnect = Some(false);
    let rendered = render_request(&request, "\n").expect("it renders");
    let again = HttpDoc::parse("feed.http", &rendered).expect("it parses back");
    let read = again.raw(0).expect("the request").request;
    assert_eq!(read.kind, "sse");
    assert_eq!(read.headers, request.headers);
    assert_eq!(read.stream, request.stream);
}
