use crate::assertions::Scripts;
use crate::body::{Body, RawLanguage};
use crate::collection::SavedRequest;
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
    pre: Option<Range<usize>>,
    pre_outer: Option<Range<usize>>,
    post: Option<Range<usize>>,
    post_outer: Option<Range<usize>>,
    content_end: usize,
    kind: String,
    marker_at: Option<usize>,
    auth_at: Option<usize>,
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
    Ok(match auth {
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
    consumed: usize,
}

fn read_preamble(source: &str, lines: &[Line<'_>]) -> CoreResult<Preamble> {
    let mut out = Preamble {
        vars: Vec::new(),
        named: None,
        pre: None,
        pre_outer: None,
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
                if !key.eq_ignore_ascii_case("name") {
                    return Err(text::unsupported(
                        line.number,
                        format_args!(
                            "Mándalo does not support the `@{key}` directive — a .http file it writes carries no directives beyond `@name`"
                        ),
                    ));
                }
                let value = rest.trim();
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
        let value_lead = after.len() - after.trim_start().len();
        let value_start = line.start + colon + 1 + value_lead;
        let value_end = line.start + colon + 1 + after.trim_end().len();
        headers.push(HeaderSpan {
            name: line.start + lead..line.start + lead + name.len(),
            value: value_start..value_end.max(value_start),
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
            .map(|(h, _)| source[..h.name.start].lines().count())
            .unwrap_or(1);
        return Err(text::unsupported(
            line,
            format_args!(
                "{GRAPHQL_MARKER} only marks a GraphQL request; {value:?} means nothing to Mándalo"
            ),
        ));
    }
    let auth_at = written.iter().position(|(k, v)| {
        k.eq_ignore_ascii_case("authorization") && auth_from_header(v).is_some()
    });

    let mut body_file = None;
    let kind = if marker_at.is_some() {
        "graphql"
    } else {
        "http"
    };
    if let Some(span) = &tail.body {
        let raw = &source[span.clone()];
        if let Some(rest) = raw.trim_start().strip_prefix('<') {
            let line = source[..span.start].lines().count();
            if rest.trim_start().starts_with('@') {
                return Err(text::unsupported(
                    line,
                    "Mándalo does not support `<@` file bodies — use `<` and keep the variables in the request",
                ));
            }
            if raw.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
                return Err(text::parse_err(
                    line,
                    "a `< file` body must be the whole body — no text may follow it",
                ));
            }
            body_file = Some(text::workspace_relative(line, rest, "the body file")?);
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
        pre: preamble.pre,
        pre_outer: preamble.pre_outer,
        post: tail.post,
        post_outer: tail.post_outer,
        content_end: tail.content_end,
        kind: kind.to_string(),
        marker_at,
        auth_at,
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

        let mut auth = Auth::None;
        if let Some(at) = block.auth_at {
            if let Some(found) = auth_from_header(&headers[at].1) {
                auth = found;
                headers.remove(at);
            }
        }
        if let Some(at) = block.marker_at {
            let at = if block.auth_at.is_some_and(|a| a < at) {
                at - 1
            } else {
                at
            };
            headers.remove(at);
        }

        let body = match (&block.body, &block.body_file) {
            (_, Some(file)) => Body::Binary {
                file: file.clone(),
                content_type: None,
            },
            (Some(span), None) if block.kind == "graphql" => {
                let (query, variables) = split_graphql(&source[span.clone()]);
                Body::Graphql { query, variables }
            }
            (Some(span), None) => {
                let raw = &source[span.clone()];
                Body::Raw {
                    language: language_for(&headers, raw),
                    text: raw.to_string(),
                }
            }
            (None, None) => Body::None,
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
        let vars = text::resolve_vars(&self.vars);
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
            other => other,
        };
        request.auth = match std::mem::take(&mut request.auth) {
            Auth::Bearer { token } => Auth::Bearer {
                token: text::substitute(&token, &vars),
            },
            Auth::Basic { username, password } => Auth::Basic {
                username: text::substitute(&username, &vars),
                password: text::substitute(&password, &vars),
            },
            other => other,
        };
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

        Ok(text::splice(&self.source, edits))
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
        let rendered = render_body(&request.body)?;
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

fn render_body(body: &Body) -> CoreResult<Option<String>> {
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
        Body::Formdata { .. } => {
            return Err(CoreError::Unsupported(
                "a .http file cannot express a multipart form-data body with per-part content types".to_string(),
            ))
        }
    })
}

/// One request as `.http` text, for a new file or a new block. Everything that
/// already exists on disk is edited in place instead, so this never reformats a
/// file a person wrote.
pub fn render_request(request: &SavedRequest, nl: &str) -> CoreResult<String> {
    if request.kind != "http" && request.kind != "graphql" {
        return Err(CoreError::Unsupported(format!(
            "a .http file holds http and graphql requests, not {:?}",
            request.kind
        )));
    }
    text::reject_inexpressible(request, "http")?;
    let mut out = String::new();
    out.push_str("### ");
    out.push_str(&request.name);
    out.push_str(nl);
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
    if let Some(body) = render_body(&request.body)? {
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
