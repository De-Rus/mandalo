use crate::assertions::Scripts;
use crate::body::{Body, FormDataRow, RawLanguage};
use crate::collection::SavedRequest;
use crate::collection::SavedStream;
use crate::error::{CoreError, CoreResult};
use crate::request::Auth;
use crate::text_format::{self as text, Line, Segment};
use std::ops::Range;

pub const EXTENSIONS: [&str; 2] = ["http", "rest"];

const METHODS: [&str; 9] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT",
];

const VERSIONS: [&str; 5] = ["HTTP/1.1", "HTTP/1.0", "HTTP/2.0", "HTTP/2", "HTTP/3"];

/// REST Client's own marker for a GraphQL request. It is consumed on the way out —
/// the header exists to tell the client what the body means, not the server.
const GRAPHQL_MARKER: &str = "X-REQUEST-TYPE";

/// `# @auth inherited` marks the Authorization line below it as a collection-wide
/// default the request did not ask for — see [`Auth::Inherited`].
const INHERITED: &str = "inherited";

/// What makes an HTTP request a server-sent-events stream. There is no marker to
/// invent: this is the header the browser sends and the header the server keys
/// off, so a request that carries it *is* an SSE request.
pub const SSE_MEDIA_TYPE: &str = "text/event-stream";

/// `# @reconnect off` turns off the automatic resume an SSE stream does by
/// default. It is a directive rather than a header because there is no header
/// for it — inventing one would put a made-up name on the wire.
const RECONNECT: &str = "reconnect";

/// The header a browser sends to resume an interrupted stream. It is a real
/// header, so it is written as one — and lifted into the model, because the
/// transport carries it forward across reconnects and would otherwise send it
/// twice.
const LAST_EVENT_ID: &str = "Last-Event-ID";

/// An `Accept` value names `text/event-stream` when any of its media types is
/// that one, `;q=` parameters and all.
pub fn accepts_event_stream(value: &str) -> bool {
    value.split(',').any(|part| {
        part.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case(SSE_MEDIA_TYPE)
    })
}

fn is_event_stream(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("accept") && accepts_event_stream(v))
}

pub fn is_http_file(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpBlock {
    pub name: String,
    pub index: usize,
    pub request: SavedRequest,
}

#[derive(Debug, Clone)]
struct HeaderSpan {
    name: Range<usize>,
    value: Range<usize>,
    line: Range<usize>,
}

#[derive(Debug, Clone)]
struct BlockSpans {
    name: String,
    name_span: Option<Range<usize>>,
    span: Range<usize>,
    method: String,
    method_span: Option<Range<usize>>,
    url_span: Range<usize>,
    url: String,
    request_line_end: usize,
    headers: Vec<HeaderSpan>,
    headers_end: usize,
    body: Option<Range<usize>>,
    body_file: Option<String>,
    formdata: Option<Vec<FormDataRow>>,
    boundary: Option<String>,
    pre: Option<Range<usize>>,
    pre_outer: Option<Range<usize>>,
    post: Option<Range<usize>>,
    post_outer: Option<Range<usize>>,
    content_end: usize,
    kind: String,
    marker_at: Option<usize>,
    auth_at: Option<usize>,
    event_id_at: Option<usize>,
    inherited_auth: bool,
    inherited_span: Option<Range<usize>>,
    auto_reconnect: bool,
    reconnect_span: Option<Range<usize>>,
}

#[derive(Debug)]
pub struct HttpDoc {
    source: String,
    stem: String,
    vars: Vec<(String, String)>,
    blocks: Vec<BlockSpans>,
    newline: &'static str,
}

fn slug(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "request".to_string()
    } else {
        trimmed
    }
}

/// `Authorization: Bearer x` and `Authorization: Basic user:pass` carry exactly what
/// a typed auth block carries, so they read back as one. Every other header — an
/// API key header included — stays a literal header, which sends the same bytes.
fn auth_from_header(value: &str) -> Option<Auth> {
    let (scheme, rest) = value.split_once(char::is_whitespace)?;
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    if scheme.eq_ignore_ascii_case("bearer") {
        return Some(Auth::Bearer {
            token: rest.to_string(),
        });
    }
    if scheme.eq_ignore_ascii_case("basic") {
        let (username, password) = rest.split_once(':')?;
        return Some(Auth::Basic {
            username: username.trim().to_string(),
            password: password.trim().to_string(),
        });
    }
    None
}

fn auth_header(auth: &Auth) -> CoreResult<Option<(String, String)>> {
    Ok(match auth.effective() {
        Auth::None => None,
        Auth::Bearer { token } => Some(("Authorization".to_string(), format!("Bearer {token}"))),
        Auth::Basic { username, password } => {
            if username.contains(':') {
                return Err(CoreError::Unsupported(format!(
                    "a basic-auth username cannot contain a colon in a .http file: {username:?}"
                )));
            }
            Some((
                "Authorization".to_string(),
                format!("Basic {username}:{password}"),
            ))
        }
        Auth::Apikey {
            key,
            value,
            placement,
        } => match placement.as_str() {
            "header" => Some((key.clone(), value.clone())),
            other => {
                return Err(CoreError::Unsupported(format!(
                    "a .http file writes an api key as a header or a query parameter, not as {other:?} — put it in the URL instead"
                )))
            }
        },
        Auth::Inherited { .. } => unreachable!("effective() peels the wrapper off"),
    })
}

fn language_for(headers: &[(String, String)], text: &str) -> RawLanguage {
    let declared = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase());
    match declared.as_deref() {
        Some(v) if v.contains("json") => RawLanguage::Json,
        Some(v) if v.contains("xml") => RawLanguage::Xml,
        Some(v) if v.contains("html") => RawLanguage::Html,
        Some(v) if v.contains("javascript") => RawLanguage::Javascript,
        Some(v) if v.starts_with("text/") => RawLanguage::Text,
        _ => match Body::raw(text) {
            Body::Raw { language, .. } => language,
            _ => RawLanguage::Text,
        },
    }
}

/// A GraphQL body is the document, then a blank line, then the variables object.
fn split_graphql(text: &str) -> (String, String) {
    let mut at = 0usize;
    while at < text.len() {
        let end = text[at..].find('\n').map(|n| at + n).unwrap_or(text.len());
        if text[at..end].trim().is_empty() {
            let rest = &text[(end + 1).min(text.len())..];
            if rest.trim_start().starts_with('{') {
                return (text[..at].trim_end().to_string(), rest.trim().to_string());
            }
        }
        at = end + 1;
    }
    (text.trim_end().to_string(), String::new())
}

struct Preamble {
    vars: Vec<(String, String)>,
    named: Option<String>,
    pre: Option<Range<usize>>,
    pre_outer: Option<Range<usize>>,
    inherited_auth: bool,
    inherited_span: Option<Range<usize>>,
    auto_reconnect: bool,
    reconnect_span: Option<Range<usize>>,
    consumed: usize,
}

fn read_preamble(source: &str, lines: &[Line<'_>]) -> CoreResult<Preamble> {
    let mut out = Preamble {
        vars: Vec::new(),
        named: None,
        pre: None,
        pre_outer: None,
        inherited_auth: false,
        inherited_span: None,
        auto_reconnect: true,
        reconnect_span: None,
        consumed: 0,
    };
    let mut index = 0usize;
    while index < lines.len() {
        let line = &lines[index];
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if text::is_comment(line.text) {
            let body = text::comment_body(line.text);
            if let Some(directive) = body.strip_prefix('@') {
                let (key, rest) = directive
                    .split_once(char::is_whitespace)
                    .unwrap_or((directive, ""));
                let value = rest.trim();
                if key.eq_ignore_ascii_case("auth") {
                    if !value.eq_ignore_ascii_case(INHERITED) {
                        return Err(text::parse_err(
                            line.number,
                            format_args!(
                                "`@auth` only takes `{INHERITED}` — write the auth itself as an Authorization header"
                            ),
                        ));
                    }
                    out.inherited_auth = true;
                    out.inherited_span = Some(line.start..line.end);
                    index += 1;
                    continue;
                }
                if key.eq_ignore_ascii_case(RECONNECT) {
                    out.auto_reconnect = match value.to_ascii_lowercase().as_str() {
                        "on" | "" => true,
                        "off" => false,
                        other => {
                            return Err(text::parse_err(
                                line.number,
                                format_args!("`@{RECONNECT}` is on or off, not {other:?}"),
                            ))
                        }
                    };
                    out.reconnect_span = Some(line.start..line.end);
                    index += 1;
                    continue;
                }
                if !key.eq_ignore_ascii_case("name") {
                    return Err(text::unsupported(
                        line.number,
                        format_args!(
                            "Mándalo does not support the `@{key}` directive — a .http file it writes carries no directives beyond `@name`, `@auth` and `@{RECONNECT}`"
                        ),
                    ));
                }
                if value.is_empty() {
                    return Err(text::parse_err(line.number, "`@name` needs a name"));
                }
                out.named = Some(value.to_string());
            }
            index += 1;
            continue;
        }
        if let Some((name, value)) = text::var_definition(line.text) {
            if !text::valid_var_name(name) {
                return Err(text::parse_err(
                    line.number,
                    format_args!("{name:?} is not a valid variable name"),
                ));
            }
            out.vars.push((name.to_string(), value.to_string()));
            index += 1;
            continue;
        }
        if trimmed.starts_with('<') && trimmed[1..].trim_start().starts_with("{%") {
            let (span, next) = text::read_script(source, lines, index)?;
            out.pre = Some(span);
            out.pre_outer = Some(line.start..lines[next - 1].end);
            index = next;
            continue;
        }
        break;
    }
    out.consumed = index;
    Ok(out)
}

struct RequestLine {
    method: String,
    method_span: Option<Range<usize>>,
    url: String,
    url_span: Range<usize>,
    end: usize,
    next: usize,
}

fn read_request_line(source: &str, lines: &[Line<'_>], index: usize) -> CoreResult<RequestLine> {
    let line = &lines[index];
    let raw = &source[line.start..line.end];
    let lead = raw.len() - raw.trim_start().len();
    let body = raw.trim();
    let mut cursor = line.start + lead;
    let mut method = "GET".to_string();
    let mut method_span = None;
    let mut rest = body;

    if METHODS.contains(&body.to_ascii_uppercase().as_str()) {
        return Err(text::parse_err(line.number, "this request line has no URL"));
    }
    if let Some((first, tail)) = body.split_once(char::is_whitespace) {
        let upper = first.to_ascii_uppercase();
        if METHODS.contains(&upper.as_str()) {
            method = upper;
            method_span = Some(cursor..cursor + first.len());
            rest = tail.trim_start();
            cursor += body.len() - rest.len();
        } else if first.len() <= 12 && first.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(text::parse_err(
                line.number,
                format_args!(
                    "{first:?} is not an HTTP method — Mándalo supports {}",
                    METHODS.join(", ")
                ),
            ));
        }
    }

    let mut url = rest.to_string();
    let mut end = cursor + rest.len();
    for version in VERSIONS {
        if let Some(head) = url.strip_suffix(version) {
            let trimmed = head.trim_end();
            end = cursor + trimmed.len();
            url = trimmed.to_string();
            break;
        }
    }
    if url.is_empty() {
        return Err(text::parse_err(line.number, "this request line has no URL"));
    }

    // A long URL wraps onto following indented lines, which is how REST Client
    // lets a query string breathe.
    let mut next = index + 1;
    while next < lines.len() {
        let candidate = &lines[next];
        if candidate.text.trim().is_empty()
            || text::is_comment(candidate.text)
            || !candidate.text.starts_with([' ', '\t'])
        {
            break;
        }
        url.push_str(candidate.text.trim());
        end = candidate.end;
        next += 1;
    }

    Ok(RequestLine {
        method,
        method_span,
        url,
        url_span: cursor..end,
        end,
        next,
    })
}

fn read_headers(
    source: &str,
    lines: &[Line<'_>],
    from: usize,
    request_line_end: usize,
) -> CoreResult<(Vec<HeaderSpan>, usize, usize)> {
    let mut headers: Vec<HeaderSpan> = Vec::new();
    let mut index = from;
    let mut end = request_line_end;
    while index < lines.len() {
        let line = &lines[index];
        let trimmed = line.text.trim_start();
        if trimmed.is_empty() {
            break;
        }
        if text::is_comment(line.text) {
            index += 1;
            continue;
        }
        if trimmed.starts_with('>') || trimmed.starts_with('<') {
            break;
        }
        let raw = &source[line.start..line.end];
        let Some(colon) = raw.find(':') else {
            return Err(text::parse_err(
                line.number,
                format_args!(
                    "expected `Name: value` or a blank line before the body, found {:?}",
                    line.text.trim()
                ),
            ));
        };
        let lead = raw.len() - raw.trim_start().len();
        let name = raw[lead..colon].trim_end();
        if !text::valid_header_name(name) {
            return Err(text::parse_err(
                line.number,
                format_args!("{name:?} is not a valid header name"),
            ));
        }
        let after = &raw[colon + 1..];
        let value = after.trim();
        let value_start = line.start + colon + 1 + (after.len() - after.trim_start().len());
        headers.push(HeaderSpan {
            name: line.start + lead..line.start + lead + name.len(),
            value: value_start..value_start + value.len(),
            line: line.start..line.end,
        });
        end = line.end;
        index += 1;
    }
    Ok((headers, index, end))
}

struct Tail {
    body: Option<Range<usize>>,
    post: Option<Range<usize>>,
    post_outer: Option<Range<usize>>,
    content_end: usize,
}

fn read_tail(
    source: &str,
    lines: &[Line<'_>],
    from: usize,
    headers_end: usize,
) -> CoreResult<Tail> {
    let mut out = Tail {
        body: None,
        post: None,
        post_outer: None,
        content_end: headers_end,
    };
    let mut index = from;
    let mut body_start = None;
    let mut body_end = headers_end;
    while index < lines.len() {
        let line = &lines[index];
        let trimmed = line.text.trim_start();
        if let Some(after) = trimmed.strip_prefix('>') {
            if !after.trim_start().starts_with("{%") {
                return Err(text::unsupported(
                    line.number,
                    "Mándalo runs inline `> {% … %}` scripts only, not a script file reference",
                ));
            }
            let (span, next) = text::read_script(source, lines, index)?;
            out.post = Some(span);
            out.post_outer = Some(line.start..lines[next - 1].end);
            out.content_end = lines[next - 1].end;
            index = next;
            continue;
        }
        if out.post.is_some() {
            if trimmed.is_empty() {
                index += 1;
                continue;
            }
            return Err(text::parse_err(
                line.number,
                "nothing may follow a `> {% … %}` response script inside a request",
            ));
        }
        if trimmed.is_empty() && body_start.is_none() {
            index += 1;
            continue;
        }
        if body_start.is_none() {
            body_start = Some(line.start);
        }
        body_end = line.end;
        index += 1;
    }
    if let Some(start) = body_start {
        let trimmed = source[start..body_end].trim_end();
        if !trimmed.is_empty() {
            out.body = Some(start..start + trimmed.len());
            out.content_end = out.content_end.max(start + trimmed.len());
        }
    }
    Ok(out)
}

fn header_param(value: &str, name: &str) -> Option<String> {
    for part in value.split(';').skip(1) {
        let part = part.trim();
        let (key, raw) = part.split_once('=')?;
        if key.trim().eq_ignore_ascii_case(name) {
            return Some(raw.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn is_multipart_formdata(value: &str) -> bool {
    value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
}

/// Which of the two form-data spellings a body is written in, decided by its own
/// first line: a boundary delimiter opens the literal wire format, anything else
/// is the field-per-line form Mándalo writes.
fn is_literal_multipart(body: &str) -> bool {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with("--"))
}

fn multipart_file_ref(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    trimmed
        .strip_prefix('<')
        .filter(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
}

/// Repeated `name = < path` lines (or multipart parts with the same name) are
/// one field holding several files — fold them so the editor shows one key.
fn push_form_row(rows: &mut Vec<FormDataRow>, row: FormDataRow) {
    if row.is_file() {
        if let Some(last) = rows.last_mut() {
            if last.is_file()
                && last.key == row.key
                && last.content_type == row.content_type
                && last.enabled == row.enabled
            {
                last.files.extend(row.files);
                return;
            }
        }
    }
    rows.push(row);
}

fn parse_multipart(body: &str, boundary: &str, first_line: usize) -> CoreResult<Vec<FormDataRow>> {
    let delimiter = format!("--{boundary}");
    let closing = format!("--{boundary}--");
    let unclosed = |line: usize| {
        text::parse_err(
            line,
            format!("this multipart body is never closed with `{closing}`"),
        )
    };
    let lines: Vec<&str> = body.lines().map(|l| l.trim_end_matches('\r')).collect();
    let at = |index: usize| first_line + index;
    let mut rows = Vec::new();
    let mut index = 0;
    while index < lines.len() && lines[index].trim() != delimiter {
        if !lines[index].trim().is_empty() {
            return Err(text::parse_err(
                at(index),
                format!("a multipart body starts at its first `{delimiter}` line — nothing may come before it"),
            ));
        }
        index += 1;
    }
    if index == lines.len() {
        return Err(text::parse_err(
            first_line,
            format!("this multipart body has no `{delimiter}` part"),
        ));
    }
    while index < lines.len() {
        let part_line = at(index);
        index += 1;
        let mut name = None;
        let mut filename = None;
        let mut content_type = None;
        loop {
            let Some(line) = lines.get(index) else {
                return Err(unclosed(part_line));
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                index += 1;
                break;
            }
            if trimmed == delimiter || trimmed == closing {
                return Err(text::parse_err(
                    at(index),
                    "a part's headers end with a blank line before its content — add one before this boundary",
                ));
            }
            let Some((key, value)) = trimmed.split_once(':') else {
                return Err(text::parse_err(
                    at(index),
                    "a part header reads `Name: value`",
                ));
            };
            let (key, value) = (key.trim(), value.trim());
            if key.eq_ignore_ascii_case("content-disposition") {
                if !value.to_ascii_lowercase().starts_with("form-data") {
                    return Err(text::unsupported(
                        at(index),
                        format!("a form part's disposition is `form-data`, not {value:?}"),
                    ));
                }
                name = header_param(value, "name");
                filename = header_param(value, "filename");
            } else if key.eq_ignore_ascii_case("content-type") {
                content_type = Some(value.to_string());
            } else {
                return Err(text::unsupported(
                    at(index),
                    format!("Mándalo reads Content-Disposition and Content-Type part headers, not {key:?}"),
                ));
            }
            index += 1;
        }
        let Some(key) = name.filter(|n| !n.is_empty()) else {
            return Err(text::parse_err(
                part_line,
                "every part needs `Content-Disposition: form-data; name=\"…\"`",
            ));
        };
        let content_first = index;
        let mut content: Vec<&str> = Vec::new();
        loop {
            let Some(line) = lines.get(index) else {
                return Err(unclosed(part_line));
            };
            let trimmed = line.trim();
            if trimmed == delimiter || trimmed == closing {
                break;
            }
            content.push(line);
            index += 1;
        }
        while content.last().is_some_and(|l| l.trim().is_empty()) {
            content.pop();
        }
        let text_content = content.join("\n");
        let row = if let Some(rest) = multipart_file_ref(&text_content) {
            if text_content.trim().lines().count() > 1 {
                return Err(text::parse_err(
                    at(content_first),
                    "a `< file` part holds only the file line",
                ));
            }
            FormDataRow {
                content_type,
                ..FormDataRow::file(
                    key,
                    text::workspace_relative(at(content_first), rest, "the form-data file")?,
                )
            }
        } else if filename.is_some() {
            return Err(text::unsupported(
                at(content_first.min(lines.len().saturating_sub(1))),
                "a file part references its file with `< path` — inline file content is not supported",
            ));
        } else if content_type.is_some() {
            return Err(text::unsupported(
                part_line,
                "Mándalo sends a text part as plain text — a per-part content type belongs on a `< file` part",
            ));
        } else {
            FormDataRow::text(key, text_content)
        };
        push_form_row(&mut rows, row);
        if lines[index].trim() == closing {
            for (extra, line) in lines.iter().enumerate().skip(index + 1) {
                if !line.trim().is_empty() {
                    return Err(text::parse_err(
                        at(extra),
                        format!("nothing may follow the closing `{closing}` line"),
                    ));
                }
            }
            return Ok(rows);
        }
    }
    Err(unclosed(first_line))
}

/// The path in a `= < ./path` value, if that is what the value is. `<` opens a
/// file reference only when whitespace or a `.` follows it, so `k = <b>bold</b>`
/// stays the text it looks like — the same rule a `< ./file` body follows.
fn form_file_ref(value: &str) -> Option<&str> {
    value
        .strip_prefix('<')
        .filter(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '.']))
}

/// The `; type=…` a file field may carry, split off the path(s) it follows.
fn split_file_ref(line: usize, rest: &str) -> CoreResult<(&str, Option<String>)> {
    let Some((path, params)) = rest.split_once(';') else {
        return Ok((rest, None));
    };
    let Some((key, value)) = params.split_once('=') else {
        return Err(text::parse_err(
            line,
            "a form file takes one parameter, written `; type=text/plain`",
        ));
    };
    if !key.trim().eq_ignore_ascii_case("type") {
        return Err(text::unsupported(
            line,
            format_args!(
                "a form file takes only `; type=…`, not {:?} — every other part header belongs on the request",
                key.trim()
            ),
        ));
    }
    let value = value.trim();
    if value.is_empty() {
        return Err(text::parse_err(line, "`; type=` needs a content type"));
    }
    Ok((path, Some(value.to_string())))
}

/// `<` opens another file on the same field when whitespace precedes it and a
/// space, tab or `.` follows — the same rule a single `< ./path` uses.
fn next_file_angle(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = s[from..].find('<') {
        let i = from + rel;
        if i > 0 {
            let before = s[..i].chars().next_back()?;
            if !before.is_whitespace() {
                from = i + 1;
                continue;
            }
        }
        let after = &s[i + 1..];
        if after.is_empty() || after.starts_with([' ', '\t', '.']) {
            return Some(i);
        }
        from = i + 1;
    }
    None
}

fn strip_leading_file_angle(s: &str) -> Option<&str> {
    let rest = s.strip_prefix('<')?;
    if rest.is_empty() || rest.starts_with([' ', '\t', '.']) {
        Some(rest)
    } else {
        None
    }
}

/// One or more `< ./path` references on a single form field line.
fn parse_file_paths(line: usize, raw: &str) -> CoreResult<Vec<String>> {
    let mut paths = Vec::new();
    let mut rest = raw.trim();
    if rest.is_empty() {
        return Err(text::parse_err(line, "the form-data file needs a path"));
    }
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(after) = strip_leading_file_angle(rest) {
            rest = after.trim_start();
        } else if !paths.is_empty() {
            return Err(text::parse_err(
                line,
                "another file on this field starts with `< ./path`",
            ));
        }
        let (path, leftover) = match next_file_angle(rest) {
            Some(i) => (rest[..i].trim(), &rest[i..]),
            None => (rest.trim(), ""),
        };
        if path.is_empty() {
            return Err(text::parse_err(line, "the form-data file needs a path"));
        }
        paths.push(text::workspace_relative(line, path, "the form-data file")?);
        rest = leftover;
    }
    Ok(paths)
}

fn parse_form_fields(body: &str, first_line: usize) -> CoreResult<Vec<FormDataRow>> {
    let mut rows = Vec::new();
    for (offset, raw) in body.lines().enumerate() {
        let line = first_line + offset;
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let Some((at, separator)) = text.char_indices().find(|(_, c)| *c == '=' || *c == '<')
        else {
            return Err(text::parse_err(
                line,
                format_args!(
                    "a form field reads `name = value`, or `name = < ./path` to send a file, not {text:?}"
                ),
            ));
        };
        let key = text[..at].trim();
        if key.is_empty() {
            return Err(text::parse_err(
                line,
                format_args!("this form field has no name before its `{separator}`"),
            ));
        }
        let value = text[at + separator.len_utf8()..].trim();
        // `name < ./path` is the shape Mándalo wrote for one release. It still reads.
        let reference = if separator == '<' {
            Some(value)
        } else {
            form_file_ref(value)
        };
        push_form_row(
            &mut rows,
            match reference {
                Some(reference) => {
                    let (paths_raw, content_type) = split_file_ref(line, reference)?;
                    let files = parse_file_paths(line, paths_raw)?;
                    FormDataRow {
                        content_type,
                        files,
                        ..FormDataRow::text(key, "")
                    }
                }
                None => FormDataRow::text(key, value),
            },
        );
    }
    Ok(rows)
}

fn reject_unwritable_field(row: &FormDataRow) -> CoreResult<()> {
    if !row.enabled {
        return Err(CoreError::Unsupported(
            "a .http file cannot keep a disabled form field — enable it or remove it".to_string(),
        ));
    }
    if row.key.trim() != row.key || row.key.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "the form field name {:?} cannot be empty or padded with spaces in a .http file",
            row.key
        )));
    }
    Ok(())
}

fn render_form_fields(rows: &[FormDataRow]) -> CoreResult<String> {
    let mut out = String::new();
    for row in rows {
        reject_unwritable_field(row)?;
        if row.key.contains(['=', '<', '\n', '\r']) {
            return Err(CoreError::Unsupported(format!(
                "the form field name {:?} cannot carry `=`, `<` or a line break in a .http file",
                row.key
            )));
        }
        if row.is_file() {
            for path in &row.files {
                if path.contains(';') {
                    return Err(CoreError::Unsupported(format!(
                        "the form file path {path:?} cannot carry a semicolon in a .http file — `;` starts the `type=` parameter"
                    )));
                }
                if path.contains('<') {
                    return Err(CoreError::Unsupported(format!(
                        "the form file path {path:?} cannot carry `<` in a .http file — `<` starts the next file on the field"
                    )));
                }
            }
            out.push_str(&row.key);
            out.push_str(" =");
            for path in &row.files {
                out.push_str(&format!(" < {path}"));
            }
            if let Some(content_type) = &row.content_type {
                out.push_str(&format!("; type={content_type}"));
            }
            out.push('\n');
        } else {
            if form_file_ref(&row.value).is_some() {
                return Err(CoreError::Unsupported(format!(
                    "the form field {:?} has a value starting with `<`, which a .http file would read back as a file reference",
                    row.key
                )));
            }
            if row.value.contains(['\n', '\r']) {
                return Err(CoreError::Unsupported(format!(
                    "the form field {:?} has a value spanning more than one line, which a .http file cannot write — keep it on one line",
                    row.key
                )));
            }
            if row.value.trim() != row.value {
                return Err(CoreError::Unsupported(format!(
                    "the form field {:?} has a value padded with spaces, which a .http file cannot write",
                    row.key
                )));
            }
            out.push_str(&format!("{} = {}\n", row.key, row.value));
        }
    }
    Ok(out.trim_end().to_string())
}

fn render_formdata(rows: &[FormDataRow], boundary: &str) -> CoreResult<String> {
    let mut out = String::new();
    for row in rows {
        reject_unwritable_field(row)?;
        if row.key.contains('"') {
            return Err(CoreError::Unsupported(format!(
                "the form field name {:?} cannot carry a double quote in a .http file",
                row.key
            )));
        }
        if row.is_file() {
            for path in &row.files {
                let file_name = path.rsplit('/').next().unwrap_or(path);
                out.push_str(&format!(
                    "--{boundary}\nContent-Disposition: form-data; name=\"{}\"; filename=\"{file_name}\"\n",
                    row.key
                ));
                if let Some(ct) = &row.content_type {
                    out.push_str(&format!("Content-Type: {ct}\n"));
                }
                out.push_str(&format!("\n< {path}\n"));
            }
        } else {
            if row.value.contains(&format!("--{boundary}")) {
                return Err(CoreError::Unsupported(format!(
                    "the form field {:?} contains the boundary line itself",
                    row.key
                )));
            }
            out.push_str(&format!(
                "--{boundary}\nContent-Disposition: form-data; name=\"{}\"\n\n{}\n",
                row.key, row.value
            ));
        }
    }
    out.push_str(&format!("--{boundary}--"));
    Ok(out)
}

fn parse_block(source: &str, segment: &Segment<'_>) -> CoreResult<BlockSpans> {
    let lines = &segment.lines;
    let preamble = read_preamble(source, lines)?;
    if preamble.consumed >= lines.len() {
        let line = lines.first().map(|l| l.number).unwrap_or(1);
        return Err(text::parse_err(
            line,
            "this block has no request line — a request needs `METHOD url`",
        ));
    }
    let request_line = read_request_line(source, lines, preamble.consumed)?;
    let (headers, after_headers, headers_end) =
        read_headers(source, lines, request_line.next, request_line.end)?;
    let tail = read_tail(source, lines, after_headers, headers_end)?;

    let written: Vec<(String, String)> = headers
        .iter()
        .map(|h| {
            (
                source[h.name.clone()].to_string(),
                source[h.value.clone()].to_string(),
            )
        })
        .collect();

    let marker_at = written.iter().position(|(k, v)| {
        k.eq_ignore_ascii_case(GRAPHQL_MARKER) && v.eq_ignore_ascii_case("graphql")
    });
    if let Some((name, value)) = written
        .iter()
        .find(|(k, v)| k.eq_ignore_ascii_case(GRAPHQL_MARKER) && !v.eq_ignore_ascii_case("graphql"))
    {
        let line = headers
            .iter()
            .zip(&written)
            .find(|(_, w)| w.0 == *name)
            .map(|(h, _)| text::line_of(lines, h.name.start))
            .unwrap_or(1);
        return Err(text::unsupported(
            line,
            format_args!(
                "{GRAPHQL_MARKER} only marks a GraphQL request; {value:?} means nothing to Mándalo"
            ),
        ));
    }
    let event_id_at = written
        .iter()
        .position(|(k, _)| k.eq_ignore_ascii_case(LAST_EVENT_ID));
    let auth_at = written.iter().position(|(k, v)| {
        k.eq_ignore_ascii_case("authorization") && auth_from_header(v).is_some()
    });

    let mut body_file = None;
    let kind = if marker_at.is_some() {
        "graphql"
    } else if is_event_stream(&written) {
        "sse"
    } else {
        "http"
    };
    if kind == "sse" {
        if request_line.method != "GET" {
            return Err(text::unsupported(
                text::line_of(lines, request_line.url_span.start),
                format_args!(
                    "server-sent events arrive over a GET, so a request that accepts {SSE_MEDIA_TYPE} cannot be a {}",
                    request_line.method
                ),
            ));
        }
        if let Some(span) = &tail.post {
            return Err(text::unsupported(
                text::line_of(lines, span.start),
                "a server-sent-events stream has no single response, so a `> {% … %}` script has nothing to run against — read the events with `mandalo listen --json` instead",
            ));
        }
    } else if let Some(span) = &preamble.reconnect_span {
        return Err(text::unsupported(
            text::line_of(lines, span.start),
            format_args!(
                "`@{RECONNECT}` only means something to a stream — add `Accept: {SSE_MEDIA_TYPE}` if this request is one"
            ),
        ));
    }
    if let Some(span) = &tail.body {
        let raw = &source[span.clone()];
        let opener = raw.trim_start();
        let line = text::line_of(lines, span.start);
        if opener.starts_with("<@") {
            return Err(text::unsupported(
                line,
                "Mándalo does not support `<@` file bodies — use `<` and keep the variables in the request",
            ));
        }
        // `<` only opens a file reference when whitespace follows it, so an XML or
        // HTML body stays a body instead of being read as a path.
        if let Some(rest) = opener
            .strip_prefix('<')
            .filter(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
        {
            if raw.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
                return Err(text::parse_err(
                    line,
                    "a `< file` body must be the whole body — no text may follow it",
                ));
            }
            body_file = Some(text::workspace_relative(line, rest, "the body file")?);
        }
    }

    let mut formdata = None;
    let mut boundary = None;
    if let (Some(span), None, "http") = (&tail.body, &body_file, kind) {
        let content_type = written
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"));
        if let Some((_, value)) = content_type.filter(|(_, v)| is_multipart_formdata(v)) {
            let line = text::line_of(lines, span.start);
            let raw = &source[span.clone()];
            let declared = header_param(value, "boundary").filter(|b| !b.is_empty());
            if is_literal_multipart(raw) {
                let found = declared.ok_or_else(|| {
                    text::parse_err(
                        line,
                        "multipart/form-data needs a `boundary=` parameter in its Content-Type",
                    )
                })?;
                formdata = Some(parse_multipart(raw, &found, line)?);
                boundary = Some(found);
            } else {
                if declared.is_some() {
                    return Err(text::parse_err(
                        line,
                        "this form body is written as `name = value` lines, which carry no boundary — remove the `boundary=` parameter, because the one on the wire is chosen when the request is sent",
                    ));
                }
                formdata = Some(parse_form_fields(raw, line)?);
            }
        }
    }

    let name = segment
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .or(preamble.named)
        .unwrap_or_else(|| format!("{} {}", request_line.method, request_line.url));

    Ok(BlockSpans {
        name,
        name_span: segment.name_span.clone(),
        span: segment.span.clone(),
        method: request_line.method,
        method_span: request_line.method_span,
        url: request_line.url,
        url_span: request_line.url_span,
        request_line_end: request_line.end,
        headers,
        headers_end,
        body: tail.body,
        body_file,
        formdata,
        boundary,
        pre: preamble.pre,
        pre_outer: preamble.pre_outer,
        post: tail.post,
        post_outer: tail.post_outer,
        content_end: tail.content_end,
        kind: kind.to_string(),
        marker_at,
        auth_at,
        event_id_at: event_id_at.filter(|_| kind == "sse"),
        inherited_auth: preamble.inherited_auth,
        inherited_span: preamble.inherited_span,
        auto_reconnect: preamble.auto_reconnect,
        reconnect_span: preamble.reconnect_span,
    })
}

impl HttpDoc {
    /// `stem` seeds the stable request ids — pass the file's path inside the
    /// collection so two files cannot mint the same id.
    pub fn parse(stem: &str, source: &str) -> CoreResult<HttpDoc> {
        let newline = text::newline_of(source);
        let parts = text::segments(source, text::lines(source));
        let mut blocks = Vec::new();
        let mut vars = Vec::new();
        for segment in &parts {
            let preamble = read_preamble(source, &segment.lines)?;
            vars.extend(preamble.vars);
            // A separator with no name and nothing under it is how people close a
            // file; a *named* one that declares no request is a mistake worth saying.
            if segment.is_declarative()
                && segment.name.as_deref().unwrap_or("").is_empty()
                && preamble.named.is_none()
            {
                continue;
            }
            blocks.push(parse_block(source, segment)?);
        }
        Ok(HttpDoc {
            source: source.to_string(),
            stem: slug(stem),
            vars,
            blocks,
            newline,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.blocks.iter().map(|b| b.name.clone()).collect()
    }

    /// The index a `path#fragment` names: a number is an index, anything else is
    /// matched against the block names.
    pub fn index_of(&self, fragment: &str) -> CoreResult<usize> {
        text::indexes_of(&self.names(), fragment, self.blocks.len())
    }

    /// The request exactly as written, `{{vars}}` and all. This is the view an
    /// editor loads: the only one that can be written back without baking the
    /// file's own variables into literals.
    pub fn raw(&self, index: usize) -> CoreResult<HttpBlock> {
        let block = self.block(index)?;
        let source = &self.source;
        let mut headers: Vec<(String, String)> = block
            .headers
            .iter()
            .map(|h| {
                (
                    source[h.name.clone()].to_string(),
                    source[h.value.clone()].to_string(),
                )
            })
            .collect();

        // The folded headers come out from the back, so one removal cannot move
        // the index of the next one.
        let mut auth = Auth::None;
        let mut sse_id = None;
        let mut lifted: Vec<usize> = [block.auth_at, block.marker_at, block.event_id_at]
            .into_iter()
            .flatten()
            .collect();
        lifted.sort_unstable();
        for at in lifted.into_iter().rev() {
            if Some(at) == block.auth_at {
                match auth_from_header(&headers[at].1) {
                    Some(found) => {
                        auth = if block.inherited_auth {
                            Auth::inherited(found)
                        } else {
                            found
                        };
                    }
                    None => continue,
                }
            } else if Some(at) == block.event_id_at {
                sse_id = Some(headers[at].1.clone());
            }
            headers.remove(at);
        }

        let body = match (&block.body, &block.body_file, &block.formdata) {
            (_, Some(file), _) => Body::Binary {
                file: file.clone(),
                content_type: None,
            },
            (Some(span), None, None) if block.kind == "graphql" => {
                let (query, variables) = split_graphql(&source[span.clone()]);
                Body::Graphql { query, variables }
            }
            (Some(_), None, Some(rows)) => {
                if let Some(at) = headers.iter().position(|(k, v)| {
                    k.eq_ignore_ascii_case("content-type") && is_multipart_formdata(v)
                }) {
                    headers.remove(at);
                }
                Body::Formdata { rows: rows.clone() }
            }
            (Some(span), None, None) => {
                let raw = &source[span.clone()];
                Body::Raw {
                    language: language_for(&headers, raw),
                    text: raw.to_string(),
                }
            }
            (None, None, _) => Body::None,
        };

        Ok(HttpBlock {
            name: block.name.clone(),
            index,
            request: SavedRequest {
                id: format!("{}-{index}", self.stem),
                name: block.name.clone(),
                kind: block.kind.clone(),
                method: block.method.clone(),
                url: block.url.clone(),
                description: None,
                headers,
                auth,
                body,
                grpc: None,
                stream: (block.kind == "sse").then(|| SavedStream {
                    auto_reconnect: (!block.auto_reconnect).then_some(false),
                    last_event_id: sse_id,
                    ..SavedStream::default()
                }),
                scripts: Scripts {
                    pre: block.pre.clone().map(|s| text::dedent(&source[s])),
                    post: block.post.clone().map(|s| text::dedent(&source[s])),
                },
                tests: Vec::new(),
                captures: Vec::new(),
            },
        })
    }

    /// The request the runner sends: the same one with the file's own `@vars`
    /// already applied, which is what makes a file-scoped name win over the
    /// environment's name.
    pub fn resolved(&self, index: usize) -> CoreResult<HttpBlock> {
        let vars = text::resolve_vars(&self.vars)?;
        let mut block = self.raw(index)?;
        if vars.is_empty() {
            return Ok(block);
        }
        let request = &mut block.request;
        request.url = text::substitute(&request.url, &vars);
        request.headers = request
            .headers
            .iter()
            .map(|(k, v)| (text::substitute(k, &vars), text::substitute(v, &vars)))
            .collect();
        request.body = match std::mem::take(&mut request.body) {
            Body::Raw { language, text: t } => Body::Raw {
                language,
                text: text::substitute(&t, &vars),
            },
            Body::Graphql { query, variables } => Body::Graphql {
                query: text::substitute(&query, &vars),
                variables: text::substitute(&variables, &vars),
            },
            Body::Binary { file, content_type } => Body::Binary {
                file: text::substitute(&file, &vars),
                content_type,
            },
            Body::Formdata { rows } => Body::Formdata {
                rows: rows
                    .into_iter()
                    .map(|row| FormDataRow {
                        key: text::substitute(&row.key, &vars),
                        value: text::substitute(&row.value, &vars),
                        files: row
                            .files
                            .iter()
                            .map(|f| text::substitute(f, &vars))
                            .collect(),
                        ..row
                    })
                    .collect(),
            },
            other => other,
        };
        request.auth = substitute_auth(std::mem::take(&mut request.auth), &vars);
        Ok(block)
    }

    pub fn all_raw(&self) -> CoreResult<Vec<HttpBlock>> {
        (0..self.blocks.len()).map(|i| self.raw(i)).collect()
    }

    fn block(&self, index: usize) -> CoreResult<&BlockSpans> {
        self.blocks
            .get(index)
            .ok_or_else(|| CoreError::NotFound(format!("there is no request {index} in this file")))
    }

    /// The headers as they sit on the wire, marker and Authorization folded back in
    /// at the positions the file wrote them.
    fn wire_headers(
        &self,
        block: &BlockSpans,
        request: &SavedRequest,
    ) -> CoreResult<Vec<(String, String)>> {
        let mut out = request.headers.clone();
        if let Some((name, value)) = auth_header(&request.auth)? {
            let at = block.auth_at.unwrap_or(out.len()).min(out.len());
            out.insert(at, (name, value));
        }
        if request.kind == "graphql" {
            let at = block.marker_at.unwrap_or(0).min(out.len());
            out.insert(at, (GRAPHQL_MARKER.to_string(), "GraphQL".to_string()));
        }
        if let Some(id) = request
            .stream
            .as_ref()
            .and_then(|s| s.last_event_id.as_ref())
        {
            let at = block.event_id_at.unwrap_or(out.len()).min(out.len());
            out.insert(at, (LAST_EVENT_ID.to_string(), id.clone()));
        }
        if matches!(request.body, Body::Formdata { .. })
            && !out
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            out.push((
                "Content-Type".to_string(),
                match block.boundary.as_deref() {
                    Some(boundary) => format!("multipart/form-data; boundary={boundary}"),
                    None => "multipart/form-data".to_string(),
                },
            ));
        }
        Ok(out)
    }

    fn written_headers(&self, block: &BlockSpans) -> Vec<(String, String)> {
        block
            .headers
            .iter()
            .map(|h| {
                (
                    self.source[h.name.clone()].to_string(),
                    self.source[h.value.clone()].to_string(),
                )
            })
            .collect()
    }

    /// Rewrites one request in place, touching only the spans whose value actually
    /// changed. A save that changed nothing returns the file byte for byte.
    pub fn replace(&self, index: usize, request: &SavedRequest) -> CoreResult<String> {
        let block = self.block(index)?;
        let current = self.raw(index)?;
        let nl = self.newline;
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();

        if request.name != current.name {
            match &block.name_span {
                Some(span) => edits.push((span.clone(), request.name.clone())),
                None => edits.push((
                    block.span.start..block.span.start,
                    format!("### {}{nl}", request.name),
                )),
            }
        }
        if request.method != current.request.method {
            match &block.method_span {
                Some(span) => edits.push((span.clone(), request.method.clone())),
                None => edits.push((
                    block.url_span.start..block.url_span.start,
                    format!("{} ", request.method),
                )),
            }
        }
        if request.url != current.request.url {
            edits.push((block.url_span.clone(), request.url.clone()));
        }

        edits.extend(self.inherited_edits(block, request));
        edits.extend(self.reconnect_edits(block, request));

        let desired = self.wire_headers(block, request)?;
        let written = self.written_headers(block);
        if desired != written {
            edits.extend(self.header_edits(block, &written, &desired));
        }

        if request.body != current.request.body {
            edits.extend(self.body_edits(block, request)?);
        }

        edits.extend(self.script_edits(
            block,
            &current.request.scripts.pre,
            &request.scripts.pre,
            true,
        ));
        edits.extend(self.script_edits(
            block,
            &current.request.scripts.post,
            &request.scripts.post,
            false,
        ));

        text::reject_inexpressible(request, "http")?;
        text::reject_description_edit(request, "http")?;
        if request.grpc.is_some() {
            return Err(CoreError::Unsupported(
                "a gRPC request belongs in a .grpc file, not a .http file".to_string(),
            ));
        }
        check_stream(request)?;

        Ok(text::splice(&self.source, edits))
    }

    /// The `# @auth inherited` line appears and disappears with the wrapper on the
    /// request's auth, so a request that stops inheriting stops saying it does.
    fn inherited_edits(
        &self,
        block: &BlockSpans,
        request: &SavedRequest,
    ) -> Vec<(Range<usize>, String)> {
        let wanted = request.auth.is_inherited();
        match (&block.inherited_span, wanted) {
            (Some(span), false) => {
                let mut end = span.end;
                while end < self.source.len() && self.source[end..].starts_with(['\r', '\n']) {
                    end += 1;
                }
                vec![(span.start..end, String::new())]
            }
            (None, true) => {
                let at = block
                    .method_span
                    .as_ref()
                    .map(|s| s.start)
                    .unwrap_or(block.url_span.start);
                vec![(at..at, format!("# @auth {INHERITED}{}", self.newline))]
            }
            _ => Vec::new(),
        }
    }

    /// The `# @reconnect off` line appears and disappears with the option it
    /// carries, so a stream that stops resuming stops saying it does.
    fn reconnect_edits(
        &self,
        block: &BlockSpans,
        request: &SavedRequest,
    ) -> Vec<(Range<usize>, String)> {
        let resumes = request.kind != "sse"
            || request
                .stream
                .as_ref()
                .and_then(|s| s.auto_reconnect)
                .unwrap_or(true);
        match (&block.reconnect_span, resumes) {
            (Some(span), _) if request.kind != "sse" => {
                let mut end = span.end;
                while end < self.source.len() && self.source[end..].starts_with(['\r', '\n']) {
                    end += 1;
                }
                vec![(span.start..end, String::new())]
            }
            (Some(span), _) if block.auto_reconnect != resumes => vec![(
                span.clone(),
                format!("# @{RECONNECT} {}", if resumes { "on" } else { "off" }),
            )],
            (None, false) => {
                let at = block
                    .method_span
                    .as_ref()
                    .map(|s| s.start)
                    .unwrap_or(block.url_span.start);
                vec![(at..at, format!("# @{RECONNECT} off{}", self.newline))]
            }
            _ => Vec::new(),
        }
    }

    /// Header names are matched in order, so a value edit stays on its own line and
    /// the comments between headers survive.
    fn header_edits(
        &self,
        block: &BlockSpans,
        written: &[(String, String)],
        desired: &[(String, String)],
    ) -> Vec<(Range<usize>, String)> {
        let nl = self.newline;
        let mut edits = Vec::new();
        let mut taken = vec![false; desired.len()];
        let mut last_line_end = block.request_line_end;
        for (position, (name, _)) in written.iter().enumerate() {
            let span = &block.headers[position];
            let matched = desired
                .iter()
                .enumerate()
                .find(|(at, (k, _))| !taken[*at] && k.eq_ignore_ascii_case(name));
            match matched {
                Some((at, (_, value))) => {
                    taken[at] = true;
                    if *value != written[position].1 {
                        edits.push((span.value.clone(), value.clone()));
                    }
                    last_line_end = span.line.end;
                }
                None => {
                    let from = last_line_end;
                    edits.push((from..span.line.end, String::new()));
                }
            }
        }
        let added: Vec<String> = desired
            .iter()
            .enumerate()
            .filter(|(at, _)| !taken[*at])
            .map(|(_, (name, value))| format!("{nl}{name}: {value}"))
            .collect();
        if !added.is_empty() {
            edits.push((last_line_end..last_line_end, added.concat()));
        }
        edits
    }

    fn body_edits(
        &self,
        block: &BlockSpans,
        request: &SavedRequest,
    ) -> CoreResult<Vec<(Range<usize>, String)>> {
        let nl = self.newline;
        let rendered = render_body(&request.body, block.boundary.as_deref())?;
        Ok(match (&block.body, rendered) {
            (Some(span), Some(text)) => vec![(span.clone(), text)],
            (Some(span), None) => vec![(block.headers_end..span.end, String::new())],
            (None, Some(text)) => vec![(
                block.headers_end..block.headers_end,
                format!("{nl}{nl}{text}"),
            )],
            (None, None) => Vec::new(),
        })
    }

    fn script_edits(
        &self,
        block: &BlockSpans,
        current: &Option<String>,
        desired: &Option<String>,
        is_pre: bool,
    ) -> Vec<(Range<usize>, String)> {
        let normalized = desired.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if current.as_deref() == normalized {
            return Vec::new();
        }
        let nl = self.newline;
        let (inner, outer) = if is_pre {
            (&block.pre, &block.pre_outer)
        } else {
            (&block.post, &block.post_outer)
        };
        match (inner, outer, normalized) {
            (Some(span), _, Some(text)) => vec![(span.clone(), format!("{nl}{text}{nl}"))],
            (_, Some(span), None) => vec![(span.clone(), String::new())],
            (None, None, Some(text)) => {
                let marker = if is_pre { '<' } else { '>' };
                let at = if is_pre {
                    block.span.start
                } else {
                    block.content_end
                };
                vec![(
                    at..at,
                    if is_pre {
                        format!("{marker} {{%{nl}{text}{nl}%}}{nl}")
                    } else {
                        format!("{nl}{nl}{marker} {{%{nl}{text}{nl}%}}")
                    },
                )]
            }
            _ => Vec::new(),
        }
    }

    /// Cuts one request out of the file, separator line included.
    pub fn remove(&self, index: usize) -> CoreResult<String> {
        let block = self.block(index)?;
        let mut end = block.span.end;
        while end < self.source.len() && self.source[end..].starts_with(['\r', '\n']) {
            end += 1;
        }
        let mut out = self.source.clone();
        out.replace_range(block.span.start..end, "");
        Ok(out)
    }

    /// Appends a request, keeping the file's own line ending.
    pub fn append(&self, request: &SavedRequest) -> CoreResult<String> {
        let nl = self.newline;
        let rendered = render_request(request, nl)?;
        let trimmed = self.source.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Ok(rendered);
        }
        Ok(format!("{trimmed}{nl}{nl}{rendered}"))
    }
}

/// A `.http` file says "this is a stream" by accepting `text/event-stream` and no
/// other way, so a request whose kind and headers disagree is a hard stop rather
/// than a file that reads back as something else.
fn check_stream(request: &SavedRequest) -> CoreResult<()> {
    let sse = request.kind == "sse";
    if sse && !is_event_stream(&request.headers) {
        return Err(CoreError::Unsupported(format!(
            "a .http file marks a stream with `Accept: {SSE_MEDIA_TYPE}` — add that header, because there is no other way to write it"
        )));
    }
    if !sse && is_event_stream(&request.headers) {
        return Err(CoreError::Unsupported(format!(
            "this request accepts {SSE_MEDIA_TYPE}, which a .http file reads back as a stream — save it as an sse request or drop the header"
        )));
    }
    if !sse && request.stream.is_some() {
        return Err(CoreError::Unsupported(
            "a websocket or an mqtt connection belongs in its own .ws or .mqtt file, not a .http file".to_string(),
        ));
    }
    let Some(stream) = request.stream.as_ref().filter(|_| sse) else {
        return Ok(());
    };
    let unwritable = [
        (!stream.subprotocols.is_empty(), "a subprotocol"),
        (stream.ping_interval_ms.is_some(), "a ping interval"),
        (stream.client_id.is_some(), "a client id"),
        (
            stream.username.is_some() || stream.password.is_some(),
            "credentials",
        ),
        (stream.clean_session.is_some(), "a clean-session flag"),
        (stream.keep_alive_secs.is_some(), "a keep-alive"),
        (!stream.subscriptions.is_empty(), "subscriptions"),
        (stream.protocol_version.is_some(), "a protocol version"),
        (!stream.messages.is_empty(), "messages to send"),
    ];
    match unwritable.iter().find(|(set, _)| *set) {
        Some((_, what)) => Err(CoreError::Unsupported(format!(
            "server-sent events travel one way over a plain GET, so this request cannot carry {what}"
        ))),
        None => Ok(()),
    }
}

fn substitute_auth(auth: Auth, vars: &std::collections::BTreeMap<String, String>) -> Auth {
    match auth {
        Auth::Inherited { auth } => Auth::inherited(substitute_auth(*auth, vars)),
        Auth::Bearer { token } => Auth::Bearer {
            token: text::substitute(&token, vars),
        },
        Auth::Basic { username, password } => Auth::Basic {
            username: text::substitute(&username, vars),
            password: text::substitute(&password, vars),
        },
        other => other,
    }
}

/// `boundary` is the one the file already wrote, and choosing it chooses the
/// spelling: a block imported in the literal form keeps it, everything else gets
/// the field-per-line form.
fn render_body(body: &Body, boundary: Option<&str>) -> CoreResult<Option<String>> {
    Ok(match body {
        Body::None => None,
        Body::Raw { text, .. } if text.trim().is_empty() => None,
        Body::Raw { text, .. } => Some(text.clone()),
        Body::Binary { file, .. } => Some(format!("< {file}")),
        Body::Graphql { query, variables } if variables.trim().is_empty() => {
            Some(query.trim_end().to_string())
        }
        Body::Graphql { query, variables } => Some(format!(
            "{}\n\n{}",
            query.trim_end(),
            variables.trim()
        )),
        Body::Urlencoded { .. } => {
            return Err(CoreError::Unsupported(
                "a .http file writes a form body as text — set Content-Type: application/x-www-form-urlencoded and write `a=1&b=2` as the body".to_string(),
            ))
        }
        Body::Formdata { rows } if rows.is_empty() => None,
        Body::Formdata { rows } => Some(match boundary {
            Some(boundary) => render_formdata(rows, boundary)?,
            None => render_form_fields(rows)?,
        }),
    })
}

/// One request as `.http` text, for a new file or a new block. Everything that
/// already exists on disk is edited in place instead, so this never reformats a
/// file a person wrote.
pub fn render_request(request: &SavedRequest, nl: &str) -> CoreResult<String> {
    if !matches!(request.kind.as_str(), "http" | "graphql" | "sse") {
        return Err(CoreError::Unsupported(format!(
            "a .http file holds http, graphql and sse requests, not {:?}",
            request.kind
        )));
    }
    text::reject_inexpressible(request, "http")?;
    check_stream(request)?;
    let mut out = String::new();
    out.push_str("### ");
    out.push_str(&request.name);
    out.push_str(nl);
    if request.auth.is_inherited() {
        out.push_str(&format!("# @auth {INHERITED}{nl}"));
    }
    if request.kind == "sse"
        && request
            .stream
            .as_ref()
            .is_some_and(|s| s.auto_reconnect == Some(false))
    {
        out.push_str(&format!("# @{RECONNECT} off{nl}"));
    }
    out.push_str(&text::description_comment(
        request.description.as_deref(),
        nl,
    ));
    if let Some(pre) = request
        .scripts
        .pre
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("< {{%{nl}{pre}{nl}%}}{nl}"));
    }
    if request.method != "GET" {
        out.push_str(&request.method);
        out.push(' ');
    }
    out.push_str(&request.url);
    out.push_str(nl);

    if request.kind == "graphql" {
        out.push_str(&format!("{GRAPHQL_MARKER}: GraphQL{nl}"));
    }
    if let Some((name, value)) = auth_header(&request.auth)? {
        out.push_str(&format!("{name}: {value}{nl}"));
    }
    for (name, value) in &request.headers {
        out.push_str(&format!("{name}: {value}{nl}"));
    }
    if let Some(id) = request
        .stream
        .as_ref()
        .and_then(|s| s.last_event_id.as_ref())
    {
        out.push_str(&format!("{LAST_EVENT_ID}: {id}{nl}"));
    }
    if matches!(request.body, Body::Formdata { .. })
        && !request
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        out.push_str(&format!("Content-Type: multipart/form-data{nl}"));
    }
    if let Some(body) = render_body(&request.body, None)? {
        out.push_str(nl);
        for line in body.split('\n') {
            out.push_str(line.trim_end_matches('\r'));
            out.push_str(nl);
        }
    }
    if let Some(post) = request
        .scripts
        .post
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push_str(nl);
        out.push_str(&format!("> {{%{nl}{post}{nl}%}}{nl}"));
    }
    Ok(out)
}

pub fn render_file(requests: &[SavedRequest], nl: &str) -> CoreResult<String> {
    let mut parts = Vec::with_capacity(requests.len());
    for request in requests {
        parts.push(render_request(request, nl)?);
    }
    Ok(parts.join(nl))
}
