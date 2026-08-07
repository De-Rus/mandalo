use crate::assertions::Scripts;
use crate::body::Body;
use crate::collection::{GrpcRequest, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::request::Auth;
use crate::text_format::{self as text, Line, Segment};
use std::ops::Range;

pub const EXTENSION: &str = "grpc";

/// The only header key that is not metadata. Kept to one on purpose: everything
/// else a gRPC call needs already fits the request line or the message.
const PROTO_KEY: &str = "proto";

/// Keys a reader could reasonably expect to be structural. Sending one as metadata
/// would look like it worked, so they are a hard stop instead.
const RESERVED_LOOKING: [&str; 7] = [
    "url",
    "service",
    "method",
    "message",
    "proto-path",
    "protos",
    "import",
];

pub fn is_grpc_file(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(EXTENSION))
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrpcBlock {
    pub name: String,
    pub index: usize,
    pub request: SavedRequest,
}

#[derive(Debug, Clone)]
struct HeaderSpan {
    name: Range<usize>,
    value: Range<usize>,
    line: Range<usize>,
    is_proto: bool,
}

#[derive(Debug, Clone)]
struct BlockSpans {
    name: String,
    name_span: Option<Range<usize>>,
    span: Range<usize>,
    target: String,
    service: String,
    method: String,
    call_span: Range<usize>,
    call_line_end: usize,
    headers: Vec<HeaderSpan>,
    headers_end: usize,
    message: Option<Range<usize>>,
    pre: Option<Range<usize>>,
    pre_outer: Option<Range<usize>>,
    post: Option<Range<usize>>,
    post_outer: Option<Range<usize>>,
    content_end: usize,
}

#[derive(Debug)]
pub struct GrpcDoc {
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
                        format_args!("Mándalo does not support the `@{key}` directive"),
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

struct CallLine {
    target: String,
    service: String,
    method: String,
    span: Range<usize>,
    end: usize,
}

/// `<target>/<package.Service>/<Method>`. That is not a Mándalo invention: the gRPC
/// wire protocol addresses a call as exactly `/package.Service/Method`, so the line
/// is the real endpoint URL and interpolates like any other URL.
fn read_call_line(source: &str, lines: &[Line<'_>], index: usize) -> CoreResult<CallLine> {
    let line = &lines[index];
    let raw = &source[line.start..line.end];
    let lead = raw.len() - raw.trim_start().len();
    let body = raw.trim();
    let start = line.start + lead;

    let Some((head, method)) = body.rsplit_once('/') else {
        return Err(text::parse_err(
            line.number,
            format_args!("expected `target/package.Service/Method`, found {body:?}"),
        ));
    };
    let Some((target, service)) = head.rsplit_once('/') else {
        return Err(text::parse_err(
            line.number,
            format_args!("{body:?} names no target — write `target/package.Service/Method`"),
        ));
    };
    for (what, value) in [("target", target), ("service", service), ("method", method)] {
        if value.trim().is_empty() {
            return Err(text::parse_err(
                line.number,
                format_args!("this call line has no {what}"),
            ));
        }
        if value.contains(char::is_whitespace) {
            return Err(text::parse_err(
                line.number,
                format_args!("the {what} must not contain whitespace: {value:?}"),
            ));
        }
    }
    Ok(CallLine {
        target: target.to_string(),
        service: service.to_string(),
        method: method.to_string(),
        span: start..start + body.len(),
        end: start + body.len(),
    })
}

fn read_headers(
    source: &str,
    lines: &[Line<'_>],
    from: usize,
    call_line_end: usize,
) -> CoreResult<(Vec<HeaderSpan>, usize, usize)> {
    let mut headers: Vec<HeaderSpan> = Vec::new();
    let mut index = from;
    let mut end = call_line_end;
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
                    "expected `proto: path` or `metadata-key: value`, found {:?}",
                    line.text.trim()
                ),
            ));
        };
        let lead = raw.len() - raw.trim_start().len();
        let name = raw[lead..colon].trim_end();
        if !text::valid_header_name(name) {
            return Err(text::parse_err(
                line.number,
                format_args!("{name:?} is not a valid metadata key"),
            ));
        }
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("grpc-") {
            return Err(text::unsupported(
                line.number,
                format_args!(
                    "gRPC reserves the `grpc-` metadata prefix for the protocol itself, so {name:?} cannot be sent"
                ),
            ));
        }
        let is_proto = lower == PROTO_KEY;
        if !is_proto && RESERVED_LOOKING.contains(&lower.as_str()) {
            return Err(text::unsupported(
                line.number,
                format_args!(
                    "{name:?} reads like a Mándalo directive but would be sent as gRPC metadata — the only reserved key is `{PROTO_KEY}:`, and the call target lives on the request line"
                ),
            ));
        }
        let after = &raw[colon + 1..];
        let value = after.trim();
        let value_start = line.start + colon + 1 + (after.len() - after.trim_start().len());
        if is_proto {
            text::workspace_relative(line.number, value, "a `proto:` path")?;
        }
        headers.push(HeaderSpan {
            name: line.start + lead..line.start + lead + name.len(),
            value: value_start..value_start + value.len(),
            line: line.start..line.end,
            is_proto,
        });
        end = line.end;
        index += 1;
    }
    Ok((headers, index, end))
}

struct Tail {
    message: Option<Range<usize>>,
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
        message: None,
        post: None,
        post_outer: None,
        content_end: headers_end,
    };
    let mut index = from;
    let mut start = None;
    let mut end = headers_end;
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
        if trimmed.is_empty() && start.is_none() {
            index += 1;
            continue;
        }
        if start.is_none() {
            start = Some(line.start);
        }
        end = line.end;
        index += 1;
    }
    if let Some(at) = start {
        let trimmed = source[at..end].trim_end();
        if !trimmed.is_empty() {
            out.message = Some(at..at + trimmed.len());
            out.content_end = out.content_end.max(at + trimmed.len());
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
            "this block has no call line — a request needs `target/package.Service/Method`",
        ));
    }
    let call = read_call_line(source, lines, preamble.consumed)?;
    let (headers, after_headers, headers_end) =
        read_headers(source, lines, preamble.consumed + 1, call.end)?;
    let tail = read_tail(source, lines, after_headers, headers_end)?;

    let name = segment
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .or(preamble.named)
        .unwrap_or_else(|| format!("{}/{}", call.service, call.method));

    Ok(BlockSpans {
        name,
        name_span: segment.name_span.clone(),
        span: segment.span.clone(),
        target: call.target,
        service: call.service,
        method: call.method,
        call_span: call.span,
        call_line_end: call.end,
        headers,
        headers_end,
        message: tail.message,
        pre: preamble.pre,
        pre_outer: preamble.pre_outer,
        post: tail.post,
        post_outer: tail.post_outer,
        content_end: tail.content_end,
    })
}

impl GrpcDoc {
    pub fn parse(stem: &str, source: &str) -> CoreResult<GrpcDoc> {
        let newline = text::newline_of(source);
        let parts = text::segments(source, text::lines(source));
        let mut blocks = Vec::new();
        let mut vars = Vec::new();
        for segment in &parts {
            let preamble = read_preamble(source, &segment.lines)?;
            vars.extend(preamble.vars);
            if segment.is_declarative()
                && segment.name.as_deref().unwrap_or("").is_empty()
                && preamble.named.is_none()
            {
                continue;
            }
            blocks.push(parse_block(source, segment)?);
        }
        Ok(GrpcDoc {
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

    pub fn index_of(&self, fragment: &str) -> CoreResult<usize> {
        text::indexes_of(&self.names(), fragment, self.blocks.len())
    }

    fn block(&self, index: usize) -> CoreResult<&BlockSpans> {
        self.blocks
            .get(index)
            .ok_or_else(|| CoreError::NotFound(format!("there is no request {index} in this file")))
    }

    pub fn raw(&self, index: usize) -> CoreResult<GrpcBlock> {
        let block = self.block(index)?;
        let source = &self.source;
        let mut proto_paths = Vec::new();
        let mut metadata = Vec::new();
        for header in &block.headers {
            let name = source[header.name.clone()].to_string();
            let value = source[header.value.clone()].to_string();
            if header.is_proto {
                proto_paths.push(value);
            } else {
                metadata.push((name, value));
            }
        }
        Ok(GrpcBlock {
            name: block.name.clone(),
            index,
            request: SavedRequest {
                id: format!("{}-{index}", self.stem),
                name: block.name.clone(),
                kind: "grpc".to_string(),
                method: "POST".to_string(),
                url: block.target.clone(),
                description: None,
                headers: Vec::new(),
                auth: Auth::None,
                body: Body::None,
                grpc: Some(GrpcRequest {
                    proto_paths,
                    service: block.service.clone(),
                    method: block.method.clone(),
                    message: block
                        .message
                        .clone()
                        .map(|s| source[s].to_string())
                        .unwrap_or_else(|| "{}".to_string()),
                    metadata,
                }),
                stream: None,
                scripts: Scripts {
                    pre: block.pre.clone().map(|s| text::dedent(&source[s])),
                    post: block.post.clone().map(|s| text::dedent(&source[s])),
                },
                tests: Vec::new(),
                captures: Vec::new(),
            },
        })
    }

    pub fn resolved(&self, index: usize) -> CoreResult<GrpcBlock> {
        let vars = text::resolve_vars(&self.vars)?;
        let mut block = self.raw(index)?;
        if vars.is_empty() {
            return Ok(block);
        }
        let request = &mut block.request;
        request.url = text::substitute(&request.url, &vars);
        if let Some(grpc) = request.grpc.as_mut() {
            grpc.message = text::substitute(&grpc.message, &vars);
            grpc.proto_paths = grpc
                .proto_paths
                .iter()
                .map(|p| text::substitute(p, &vars))
                .collect();
            grpc.metadata = grpc
                .metadata
                .iter()
                .map(|(k, v)| (text::substitute(k, &vars), text::substitute(v, &vars)))
                .collect();
        }
        Ok(block)
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

    fn wire_headers(request: &SavedRequest) -> CoreResult<Vec<(String, String)>> {
        let grpc = request.grpc.as_ref().ok_or_else(|| {
            CoreError::Unsupported("a .grpc file holds gRPC requests only".to_string())
        })?;
        let mut out: Vec<(String, String)> = grpc
            .proto_paths
            .iter()
            .map(|p| (PROTO_KEY.to_string(), p.clone()))
            .collect();
        out.extend(grpc.metadata.iter().cloned());
        Ok(out)
    }

    /// Rewrites one request in place, touching only what changed.
    pub fn replace(&self, index: usize, request: &SavedRequest) -> CoreResult<String> {
        let block = self.block(index)?;
        let current = self.raw(index)?;
        let nl = self.newline;
        let grpc = request.grpc.as_ref().ok_or_else(|| {
            CoreError::Unsupported(
                "a .grpc file holds gRPC requests only — this one carries no gRPC call".to_string(),
            )
        })?;
        if request.kind != "grpc" {
            return Err(CoreError::Unsupported(format!(
                "a .grpc file holds gRPC requests, not {:?}",
                request.kind
            )));
        }
        text::reject_inexpressible(request, "grpc")?;
        text::reject_description_edit(request, "grpc")?;
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
        let call = format!("{}/{}/{}", request.url, grpc.service, grpc.method);
        if call != format!("{}/{}/{}", block.target, block.service, block.method) {
            edits.push((block.call_span.clone(), call));
        }

        let desired = GrpcDoc::wire_headers(request)?;
        let written = self.written_headers(block);
        if desired != written {
            edits.extend(self.header_edits(block, &written, &desired));
        }

        let message = grpc.message.trim();
        let current_message = current
            .request
            .grpc
            .as_ref()
            .map(|g| g.message.clone())
            .unwrap_or_default();
        if message != current_message.trim() {
            edits.push(match &block.message {
                Some(span) => (span.clone(), message.to_string()),
                None => (
                    block.headers_end..block.headers_end,
                    format!("{nl}{nl}{message}"),
                ),
            });
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

        Ok(text::splice(&self.source, edits))
    }

    fn header_edits(
        &self,
        block: &BlockSpans,
        written: &[(String, String)],
        desired: &[(String, String)],
    ) -> Vec<(Range<usize>, String)> {
        let nl = self.newline;
        let mut edits = Vec::new();
        let mut taken = vec![false; desired.len()];
        let mut last_line_end = block.call_line_end;
        for (position, (name, value)) in written.iter().enumerate() {
            let span = &block.headers[position];
            let matched = desired
                .iter()
                .enumerate()
                .find(|(at, (k, _))| !taken[*at] && k.eq_ignore_ascii_case(name));
            match matched {
                Some((at, (_, wanted))) => {
                    taken[at] = true;
                    if wanted != value {
                        edits.push((span.value.clone(), wanted.clone()));
                    }
                    last_line_end = span.line.end;
                }
                None => edits.push((last_line_end..span.line.end, String::new())),
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
                let at = if is_pre {
                    block.span.start
                } else {
                    block.content_end
                };
                vec![(
                    at..at,
                    if is_pre {
                        format!("< {{%{nl}{text}{nl}%}}{nl}")
                    } else {
                        format!("{nl}{nl}> {{%{nl}{text}{nl}%}}")
                    },
                )]
            }
            _ => Vec::new(),
        }
    }

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

pub fn render_request(request: &SavedRequest, nl: &str) -> CoreResult<String> {
    if request.kind != "grpc" {
        return Err(CoreError::Unsupported(format!(
            "a .grpc file holds gRPC requests, not {:?}",
            request.kind
        )));
    }
    let grpc = request.grpc.as_ref().ok_or_else(|| {
        CoreError::Unsupported("this gRPC request carries no gRPC call".to_string())
    })?;
    text::reject_inexpressible(request, "grpc")?;
    let mut out = String::new();
    out.push_str(&format!("### {}{nl}", request.name));
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
    out.push_str(&format!(
        "{}/{}/{}{nl}",
        request.url, grpc.service, grpc.method
    ));
    for path in &grpc.proto_paths {
        out.push_str(&format!("{PROTO_KEY}: {path}{nl}"));
    }
    for (key, value) in &grpc.metadata {
        out.push_str(&format!("{key}: {value}{nl}"));
    }
    let message = grpc.message.trim();
    if !message.is_empty() {
        out.push_str(nl);
        for line in message.split('\n') {
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
        out.push_str(&format!("{nl}> {{%{nl}{post}{nl}%}}{nl}"));
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
