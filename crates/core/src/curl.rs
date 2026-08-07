//! `curl` in and out. Both directions are pure: no file is read, no request is
//! sent, and a flag Mándalo does not implement stops the parse instead of being
//! dropped — a curl line that silently loses `--compressed` or `-x proxy` is a
//! different request wearing the same name.

use crate::body::{self, Body, FormRow, RawLanguage};
use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use crate::request::{self, Auth, RequestSpec};

/// Renders a request as a copy-pasteable `curl` command line, with every
/// `{{variable}}` resolved from the spec's own vars — an unresolved one fails
/// here exactly as it would on a send, because a command line carrying
/// `{{token}}` is not one anybody can paste.
pub fn to_curl(spec: &RequestSpec) -> CoreResult<String> {
    match spec.kind.as_str() {
        "" | "http" | "graphql" => {}
        other => {
            return Err(CoreError::Unsupported(format!(
                "a {other} request cannot be written as a curl command"
            )))
        }
    }
    let vars = &spec.vars;
    let mut url = interpolate::apply(&spec.url, vars)?;

    let mut headers = Vec::with_capacity(spec.headers.len());
    for (k, v) in &spec.headers {
        headers.push((interpolate::apply(k, vars)?, interpolate::apply(v, vars)?));
    }

    // Basic auth becomes `-u`, which is what a reader expects to see; everything
    // else goes through the same seam the wire does, so the command carries
    // exactly the header the send would.
    let resolved = request::resolve_auth(&spec.auth, vars)?;
    let (user, auth_headers, auth_query) = match resolved {
        Some(Auth::Basic { username, password }) => (
            Some(format!("{username}:{password}")),
            Vec::new(),
            Vec::new(),
        ),
        other => {
            let (auth_headers, auth_query) = request::auth_parts(other)?;
            (None, auth_headers, auth_query)
        }
    };
    for (name, _) in &auth_headers {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }
    headers.extend(auth_headers);
    if !auth_query.is_empty() {
        url = with_query(&url, &auth_query)?;
    }

    let (body_args, content_type) = body_args(spec)?;
    if matches!(spec.body, Body::Formdata { .. }) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
    }
    if let Some(ct) = content_type {
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".to_string(), ct));
        }
    }

    let method = match &spec.body {
        Body::Graphql { .. } => "POST".to_string(),
        _ if spec.method.trim().is_empty() => "GET".to_string(),
        _ => spec.method.to_uppercase(),
    };

    let mut lines = vec![format!("curl -X {method} {}", quote(&url))];
    if let Some(user) = user {
        lines.push(format!("-u {}", quote(&user)));
    }
    for (k, v) in &headers {
        lines.push(format!("-H {}", quote(&format!("{k}: {v}"))));
    }
    lines.extend(body_args);
    Ok(lines.join(" \\\n  "))
}

fn body_args(spec: &RequestSpec) -> CoreResult<(Vec<String>, Option<String>)> {
    let vars = &spec.vars;
    Ok(match &spec.body {
        Body::None => (Vec::new(), None),
        Body::Raw { language, text } => (
            vec![format!("-d {}", quote(&interpolate::apply(text, vars)?))],
            Some(language.content_type().to_string()),
        ),
        Body::Urlencoded { rows } => {
            let encoded = String::from_utf8(body::encode_urlencoded(rows, vars)?)
                .map_err(|e| CoreError::Parse(format!("form body is not text: {e}")))?;
            (
                vec![format!("-d {}", quote(&encoded))],
                Some("application/x-www-form-urlencoded".to_string()),
            )
        }
        Body::Formdata { rows } => {
            let mut args = Vec::new();
            for row in rows.iter().filter(|r| r.enabled) {
                let key = interpolate::apply(&row.key, vars)?;
                if !row.is_file() {
                    args.push(format!(
                        "-F {}",
                        quote(&format!("{key}={}", interpolate::apply(&row.value, vars)?))
                    ));
                    continue;
                }
                for path in &row.files {
                    let path = interpolate::apply(path, vars)?;
                    let field = match &row.content_type {
                        Some(ct) => format!("{key}=@{path};type={ct}"),
                        None => format!("{key}=@{path}"),
                    };
                    args.push(format!("-F {}", quote(&field)));
                }
            }
            (args, None)
        }
        // The content type is whatever the request declares. Sniffing it the way
        // a send does would mean reading the file, and this function reads nothing.
        Body::Binary { file, content_type } => (
            vec![format!(
                "--data-binary {}",
                quote(&format!("@{}", interpolate::apply(file, vars)?))
            )],
            content_type.clone(),
        ),
        Body::Graphql { query, variables } => {
            let payload = request::graphql_payload(query, variables, vars)?;
            let text = String::from_utf8(payload)
                .map_err(|e| CoreError::Parse(format!("graphql body is not text: {e}")))?;
            (
                vec![format!("-d {}", quote(&text))],
                Some("application/json".to_string()),
            )
        }
    })
}

fn with_query(url: &str, pairs: &[(String, String)]) -> CoreResult<String> {
    let mut parsed = reqwest::Url::parse(url)
        .map_err(|e| CoreError::Request(format!("invalid url {url:?}: {e}")))?;
    parsed.query_pairs_mut().extend_pairs(pairs);
    Ok(parsed.to_string())
}

/// Single quotes survive everything a shell would otherwise expand — `$`, a
/// backtick, a newline inside a JSON body.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

const SUPPORTED_FLAGS: &str =
    "-X/--request, -H/--header, -d/--data/--data-raw/--data-binary, -u/--user and --url";

/// Parses a pasted `curl` command line into a request.
pub fn from_curl(command: &str) -> CoreResult<RequestSpec> {
    let tokens = tokenize(command)?;
    let mut tokens = tokens.into_iter();
    match tokens.next().as_deref() {
        Some("curl") => {}
        Some(other) => {
            return Err(CoreError::Parse(format!(
                "a curl command has to start with `curl`, not {other:?}"
            )))
        }
        None => {
            return Err(CoreError::Parse(
                "there is no curl command here".to_string(),
            ))
        }
    }

    let mut method: Option<String> = None;
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut user: Option<String> = None;
    let mut url: Option<String> = None;

    while let Some(token) = tokens.next() {
        if !token.starts_with('-') || token == "-" {
            if let Some(first) = url.replace(token.clone()) {
                return Err(CoreError::Parse(format!(
                    "this command names two urls, {first:?} and {token:?} — Mándalo cannot turn that into one request"
                )));
            }
            continue;
        }
        let (flag, attached) = split_flag(&token);
        let mut value = || match attached.clone() {
            Some(value) => Ok(value),
            None => tokens.next().ok_or_else(|| {
                CoreError::Parse(format!("{flag} was given nothing to use as its value"))
            }),
        };
        match flag {
            "-X" | "--request" => method = Some(value()?),
            "-H" | "--header" => headers.push(header(&value()?)?),
            "-u" | "--user" => user = Some(value()?),
            "--url" => {
                let found = value()?;
                if let Some(first) = url.replace(found.clone()) {
                    return Err(CoreError::Parse(format!(
                        "this command names two urls, {first:?} and {found:?} — Mándalo cannot turn that into one request"
                    )));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" => {
                let found = value()?;
                // `--data-raw` is the one data flag curl never treats as a file
                // name, so a body that genuinely starts with `@` stays a body.
                match found.starts_with('@') && flag != "--data-raw" {
                    true => files.push(found[1..].to_string()),
                    false => data.push(found),
                }
            }
            other => {
                return Err(CoreError::Unsupported(format!(
                    "curl flag {other} is not supported — Mándalo reads {SUPPORTED_FLAGS}, and refuses to import a command it would send differently"
                )))
            }
        }
    }

    let url = url.ok_or_else(|| {
        CoreError::Parse("this curl command has no url to send anything to".to_string())
    })?;
    // curl falls back to http:// for a bare host. Guessing that for someone is
    // how a request meant for https quietly goes out in the clear.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CoreError::Parse(format!(
            "{url:?} has no scheme — write it as https://{url}, because Mándalo will not guess between http and https"
        )));
    }
    if !files.is_empty() && !data.is_empty() {
        return Err(CoreError::Unsupported(
            "this command mixes an inline body with a file body — Mándalo sends one or the other"
                .to_string(),
        ));
    }
    if files.len() > 1 {
        return Err(CoreError::Unsupported(format!(
            "this command sends {} files as one body — Mándalo sends a single file",
            files.len()
        )));
    }

    let auth = match user {
        None => Auth::None,
        Some(pair) => {
            let (username, password) = pair.split_once(':').unwrap_or((pair.as_str(), ""));
            Auth::Basic {
                username: username.to_string(),
                password: password.to_string(),
            }
        }
    };

    let has_body = !data.is_empty() || !files.is_empty();
    let body = if let Some(file) = files.pop() {
        let content_type = take_content_type(&mut headers);
        Body::Binary { file, content_type }
    } else if data.is_empty() {
        Body::None
    } else {
        // Repeated `-d` is one body joined by `&`, which is what curl sends.
        let text = data.join("&");
        match declared_content_type(&headers).as_deref() {
            Some("application/x-www-form-urlencoded") => {
                take_content_type(&mut headers);
                Body::Urlencoded {
                    rows: decode_urlencoded(&text)?,
                }
            }
            Some(other) => match raw_language(other) {
                Some(language) => {
                    take_content_type(&mut headers);
                    Body::Raw { language, text }
                }
                None => Body::raw(text),
            },
            None => Body::raw(text),
        }
    };

    let method = match method {
        Some(method) => method.to_uppercase(),
        // curl's own rule: a body with no method is a POST.
        None if has_body => "POST".to_string(),
        None => "GET".to_string(),
    };

    Ok(RequestSpec {
        kind: "http".to_string(),
        method,
        url,
        headers,
        body,
        auth,
        ..RequestSpec::default()
    })
}

/// `-XPOST`, `--header=a: b` and the plain two-token forms all reach the same
/// place: curl accepts every one of them, so a paste may carry any.
fn split_flag(token: &str) -> (&str, Option<String>) {
    if token.starts_with("--") {
        return match token.split_once('=') {
            Some((name, value)) => (name, Some(value.to_string())),
            None => (token, None),
        };
    }
    let split = token
        .char_indices()
        .nth(2)
        .map(|(i, _)| i)
        .unwrap_or(token.len());
    let (flag, rest) = token.split_at(split);
    (flag, (!rest.is_empty()).then(|| rest.to_string()))
}

fn header(raw: &str) -> CoreResult<(String, String)> {
    let (name, value) = raw.split_once(':').ok_or_else(|| {
        CoreError::Parse(format!(
            "{raw:?} is not a header — a header is written as \"Name: value\""
        ))
    })?;
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Parse(format!(
            "{raw:?} is a header with no name"
        )));
    }
    Ok((name.to_string(), value.trim().to_string()))
}

fn declared_content_type(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| {
            v.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
}

/// Removes the content type from the headers and hands it back: once it is part
/// of the body — a raw language, a binary body's type — keeping the header too
/// would send it twice on the next render.
fn take_content_type(headers: &mut Vec<(String, String)>) -> Option<String> {
    let index = headers
        .iter()
        .position(|(k, _)| k.eq_ignore_ascii_case("content-type"))?;
    Some(headers.remove(index).1)
}

fn raw_language(content_type: &str) -> Option<RawLanguage> {
    [
        RawLanguage::Json,
        RawLanguage::Text,
        RawLanguage::Xml,
        RawLanguage::Html,
        RawLanguage::Javascript,
    ]
    .into_iter()
    .find(|language| language.content_type() == content_type)
}

fn decode_urlencoded(text: &str) -> CoreResult<Vec<FormRow>> {
    let mut rows = Vec::new();
    for pair in text.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        rows.push(FormRow::new(
            decode_form_component(key)?,
            decode_form_component(value)?,
        ));
    }
    Ok(rows)
}

fn decode_form_component(raw: &str) -> CoreResult<String> {
    percent_encoding::percent_decode_str(&raw.replace('+', " "))
        .decode_utf8()
        .map(|decoded| decoded.into_owned())
        .map_err(|e| CoreError::Parse(format!("{raw:?} is not valid percent-encoded text: {e}")))
}

/// Splits a command line the way a POSIX shell would: single quotes are literal,
/// double quotes keep `\` escapes, and a `\` before a newline continues the line.
fn tokenize(source: &str) -> CoreResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(escaped) => {
                    started = true;
                    current.push(escaped);
                }
                None => return Err(unterminated("a trailing backslash")),
            },
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(inner) => current.push(inner),
                        None => return Err(unterminated("an unclosed single quote")),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('\n') => {}
                            Some(escaped @ ('"' | '\\' | '$' | '`')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(unterminated("an unclosed double quote")),
                        },
                        Some(inner) => current.push(inner),
                        None => return Err(unterminated("an unclosed double quote")),
                    }
                }
            }
            other => {
                started = true;
                current.push(other);
            }
        }
    }
    if started {
        tokens.push(current);
    }
    Ok(tokens)
}

fn unterminated(what: &str) -> CoreError {
    CoreError::Parse(format!(
        "this curl command ends with {what}, so Mándalo cannot tell where its last argument stops"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: serde_json::Value) -> RequestSpec {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn a_get_renders_as_one_line() {
        let rendered = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "get", "url": "{{base}}/users",
            "vars": {"base": "https://x.dev"}
        })))
        .unwrap();
        assert_eq!(rendered, "curl -X GET 'https://x.dev/users'");
    }

    #[test]
    fn headers_body_and_content_type_are_rendered() {
        let rendered = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "POST", "url": "https://x.dev/users",
            "headers": [["X-Trace", "{{t}}"]],
            "body": {"mode": "raw", "language": "json", "text": "{\"name\": \"{{who}}\"}"},
            "vars": {"t": "abc", "who": "ada"}
        })))
        .unwrap();
        assert_eq!(
            rendered,
            "curl -X POST 'https://x.dev/users' \\\n  \
             -H 'X-Trace: abc' \\\n  \
             -H 'Content-Type: application/json' \\\n  \
             -d '{\"name\": \"ada\"}'"
        );
    }

    #[test]
    fn basic_auth_renders_as_user_and_everything_else_as_a_header() {
        let basic = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "basic", "username": "u", "password": "p"}
        })))
        .unwrap();
        assert!(basic.contains("-u 'u:p'"), "{basic}");

        let bearer = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "bearer", "token": "{{t}}"}, "vars": {"t": "s3cret"}
        })))
        .unwrap();
        assert!(
            bearer.contains("-H 'Authorization: Bearer s3cret'"),
            "{bearer}"
        );

        let query = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev/s?q=1",
            "auth": {"type": "apikey", "key": "api_key", "value": "abc", "placement": "query"}
        })))
        .unwrap();
        assert!(
            query.contains("'https://x.dev/s?q=1&api_key=abc'"),
            "{query}"
        );
    }

    #[test]
    fn inherited_auth_is_applied_like_the_auth_it_wraps() {
        let rendered = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "inherited", "auth": {"type": "bearer", "token": "t"}}
        })))
        .unwrap();
        assert!(
            rendered.contains("-H 'Authorization: Bearer t'"),
            "{rendered}"
        );
    }

    #[test]
    fn every_body_mode_renders_the_flag_curl_uses_for_it() {
        let form = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "POST", "url": "https://x.dev",
            "body": {"mode": "formdata", "rows": [
                {"key": "note", "value": "hi"},
                {"key": "avatar", "files": ["files/a.png"], "contentType": "image/png"},
                {"key": "skip", "value": "no", "enabled": false}
            ]}
        })))
        .unwrap();
        assert!(form.contains("-F 'note=hi'"), "{form}");
        assert!(
            form.contains("-F 'avatar=@files/a.png;type=image/png'"),
            "{form}"
        );
        assert!(!form.contains("skip"), "{form}");

        let binary = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "PUT", "url": "https://x.dev",
            "body": {"mode": "binary", "file": "files/a.png", "contentType": "image/png"}
        })))
        .unwrap();
        assert!(binary.contains("--data-binary '@files/a.png'"), "{binary}");
        assert!(binary.contains("-H 'Content-Type: image/png'"), "{binary}");

        let graphql = to_curl(&spec(serde_json::json!({
            "kind": "graphql", "method": "GET", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        })))
        .unwrap();
        assert!(graphql.starts_with("curl -X POST"), "{graphql}");
        assert!(
            graphql.contains("-d '{\"query\":\"{ ping }\",\"variables\":{}}'"),
            "{graphql}"
        );
    }

    #[test]
    fn a_quote_in_a_value_survives_the_shell() {
        let rendered = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "POST", "url": "https://x.dev",
            "body": {"mode": "raw", "language": "text", "text": "it's $HOME"}
        })))
        .unwrap();
        assert!(rendered.contains("-d 'it'\\''s $HOME'"), "{rendered}");
        let parsed = from_curl(&rendered).unwrap();
        assert_eq!(parsed.body.as_text(), Some("it's $HOME"));
    }

    #[test]
    fn an_unresolved_variable_fails_loud() {
        let err = to_curl(&spec(serde_json::json!({
            "kind": "http", "method": "GET", "url": "{{base}}/users"
        })))
        .unwrap_err();
        assert_eq!(err.code(), "E_UNRESOLVED_VAR");
    }

    #[test]
    fn a_grpc_request_cannot_be_a_curl_command() {
        let err = to_curl(&spec(serde_json::json!({
            "kind": "grpc", "method": "GET", "url": "https://x.dev"
        })))
        .unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        assert!(err.to_string().contains("a grpc request cannot"), "{err}");
    }

    #[test]
    fn a_bare_url_is_a_get() {
        let parsed = from_curl("curl https://x.dev/users").unwrap();
        assert_eq!(parsed.kind, "http");
        assert_eq!(parsed.method, "GET");
        assert_eq!(parsed.url, "https://x.dev/users");
        assert_eq!(parsed.body, Body::None);
    }

    #[test]
    fn flags_are_read_attached_split_and_with_an_equals_sign() {
        for command in [
            "curl -XPOST -H'X-A: 1' --url https://x.dev -d 'body'",
            "curl -X POST -H 'X-A: 1' --url=https://x.dev --data body",
            "curl --request POST --header \"X-A: 1\" https://x.dev --data-raw body",
        ] {
            let parsed = from_curl(command).unwrap();
            assert_eq!(parsed.method, "POST", "{command}");
            assert_eq!(parsed.url, "https://x.dev", "{command}");
            assert_eq!(parsed.headers, vec![("X-A".to_string(), "1".to_string())]);
            assert_eq!(parsed.body.as_text(), Some("body"), "{command}");
        }
    }

    #[test]
    fn data_without_a_method_is_a_post_and_repeated_data_joins() {
        let parsed = from_curl("curl https://x.dev -d a=1 -d b=2").unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.body.as_text(), Some("a=1&b=2"));
    }

    #[test]
    fn user_becomes_basic_auth() {
        let parsed = from_curl("curl -u 'ada:lovelace' https://x.dev").unwrap();
        assert_eq!(
            parsed.auth,
            Auth::Basic {
                username: "ada".to_string(),
                password: "lovelace".to_string()
            }
        );
    }

    #[test]
    fn a_content_type_becomes_the_body_language_and_stops_being_a_header() {
        let parsed =
            from_curl("curl https://x.dev -H 'Content-Type: application/json' -d '{\"a\":1}'")
                .unwrap();
        assert!(parsed.headers.is_empty());
        assert_eq!(
            parsed.body,
            Body::Raw {
                language: RawLanguage::Json,
                text: "{\"a\":1}".to_string()
            }
        );
    }

    #[test]
    fn a_form_content_type_becomes_editable_rows() {
        let parsed = from_curl(
            "curl https://x.dev -H 'Content-Type: application/x-www-form-urlencoded' -d 'user=ada+lovelace&q=a%26b'",
        )
        .unwrap();
        assert_eq!(
            parsed.body,
            Body::Urlencoded {
                rows: vec![
                    FormRow::new("user", "ada lovelace"),
                    FormRow::new("q", "a&b")
                ]
            }
        );
    }

    #[test]
    fn a_data_file_becomes_a_binary_body() {
        let parsed = from_curl(
            "curl -X PUT https://x.dev -H 'Content-Type: image/png' --data-binary '@files/a.png'",
        )
        .unwrap();
        assert_eq!(
            parsed.body,
            Body::Binary {
                file: "files/a.png".to_string(),
                content_type: Some("image/png".to_string())
            }
        );
        assert!(parsed.headers.is_empty());
    }

    #[test]
    fn data_raw_keeps_a_leading_at_sign_as_a_body() {
        let parsed = from_curl("curl https://x.dev --data-raw '@notafile'").unwrap();
        assert_eq!(parsed.body.as_text(), Some("@notafile"));
    }

    #[test]
    fn an_unsupported_flag_stops_the_parse() {
        for (command, flag) in [
            ("curl --compressed https://x.dev", "--compressed"),
            ("curl -x http://proxy https://x.dev", "-x"),
            ("curl -F 'a=@b' https://x.dev", "-F"),
            ("curl --insecure https://x.dev", "--insecure"),
        ] {
            let err = from_curl(command).unwrap_err();
            assert_eq!(err.code(), "E_UNSUPPORTED", "{command}");
            assert!(err.to_string().contains(flag), "{err}");
        }
    }

    #[test]
    fn malformed_commands_fail_loud() {
        for command in [
            "wget https://x.dev",
            "curl -X POST",
            "curl https://x.dev -H 'not-a-header'",
            "curl https://x.dev -X",
            "curl 'https://x.dev",
            "curl https://x.dev https://y.dev",
            "curl https://x.dev -d inline --data-binary @file",
            "curl x.dev/users",
        ] {
            let err = from_curl(command).unwrap_err();
            assert!(
                matches!(err.code(), "E_PARSE" | "E_UNSUPPORTED"),
                "{command}: {err}"
            );
        }
    }

    #[test]
    fn a_multi_line_paste_is_read_as_one_command() {
        let parsed =
            from_curl("curl -X POST 'https://x.dev/users' \\\n  -H 'X-A: 1' \\\n  -d '{\"a\": 1}'")
                .unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.headers, vec![("X-A".to_string(), "1".to_string())]);
        assert_eq!(parsed.body.as_text(), Some("{\"a\": 1}"));
    }

    #[test]
    fn render_and_parse_round_trip() {
        for body in [
            serde_json::json!({"mode": "raw", "language": "json", "text": "{\"a\": 1}"}),
            serde_json::json!({"mode": "urlencoded", "rows": [{"key": "q", "value": "a b"}]}),
            serde_json::json!({"mode": "binary", "file": "files/a.png", "contentType": "image/png"}),
        ] {
            let original = spec(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev/users",
                "headers": [["X-Trace", "abc"]],
                "auth": {"type": "basic", "username": "ada", "password": "l0ve"},
                "body": body
            }));
            let rendered = to_curl(&original).unwrap();
            let parsed = from_curl(&rendered).unwrap();
            assert_eq!(parsed, original, "{rendered}");
            assert_eq!(to_curl(&parsed).unwrap(), rendered);
        }
    }

    #[test]
    fn a_formdata_render_survives_a_second_render() {
        let original = spec(serde_json::json!({
            "kind": "http", "method": "POST", "url": "https://x.dev",
            "body": {"mode": "formdata", "rows": [{"key": "a", "value": "1"}]}
        }));
        let rendered = to_curl(&original).unwrap();
        let err = from_curl(&rendered).unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        assert!(err.to_string().contains("-F"), "{err}");
    }
}
