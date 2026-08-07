use crate::assertions::Scripts;
use crate::body::Body;
use crate::collection::{self, SavedMessage, SavedRequest, SavedStream};
use crate::error::{CoreError, CoreResult};
use crate::request::Auth;
use crate::stream::{MqttVersion, Outgoing, Subscription};
use crate::text_format::{self as text, Line, Segment};
use std::ops::Range;

pub const WS_EXTENSION: &str = "ws";
pub const MQTT_EXTENSION: &str = "mqtt";

/// Which of the two socket formats a file is. They share every line of parsing
/// and differ only in which keys are reserved, so they share one module.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flavor {
    Ws,
    Mqtt,
}

impl Flavor {
    pub fn extension(self) -> &'static str {
        match self {
            Flavor::Ws => WS_EXTENSION,
            Flavor::Mqtt => MQTT_EXTENSION,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Flavor::Ws => "websocket",
            Flavor::Mqtt => "mqtt",
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            Flavor::Ws => "WS",
            Flavor::Mqtt => "MQTT",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Flavor::Ws => "a websocket",
            Flavor::Mqtt => "an mqtt connection",
        }
    }
}

pub fn is_ws_file(name: &str) -> bool {
    has_extension(name, WS_EXTENSION)
}

pub fn is_mqtt_file(name: &str) -> bool {
    has_extension(name, MQTT_EXTENSION)
}

fn has_extension(name: &str, extension: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(extension))
}

/// The plural spellings that name the same key as their singular. Case and the
/// dashes are folded by [`canon`]; these two are the only pairs that are not
/// just a different way of writing the same letters.
const ALIASES: [(&str, &str); 2] = [
    ("subprotocols", "subprotocol"),
    ("subscriptions", "subscribe"),
];

/// `client-id` and `clientId` are the same key. Comparing the folded spelling is
/// what lets a file be written either way and still edited in place.
fn canon(key: &str) -> String {
    let folded: String = key
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect();
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == folded)
        .map(|(_, key)| (*key).to_string())
        .unwrap_or(folded)
}

fn same_key(a: &str, b: &str) -> bool {
    canon(a) == canon(b)
}

/// The connection keys `.ws` reserves. Everything else on a `.ws` connection line
/// is an HTTP header, sent with the handshake exactly as written.
const WS_KEYS: [&str; 3] = ["subprotocol", "auto-reconnect", "ping-interval"];

/// The connection keys `.mqtt` reserves — and the whole set it accepts. MQTT has
/// no headers, so a line that is not one of these is a mistake, not a header.
const MQTT_KEYS: [&str; 7] = [
    "client-id",
    "username",
    "password",
    "keep-alive",
    "clean-session",
    "protocol-version",
    "subscribe",
];

/// Keys a reader could reasonably expect a `.ws` file to act on. Sending one as
/// an HTTP header would look like it worked, so they are a hard stop.
const WS_REFUSED: [&str; 10] = [
    "url",
    "message",
    "send",
    "topic",
    "qos",
    "retain",
    "client-id",
    "keep-alive",
    "clean-session",
    "subscribe",
];

const MESSAGE_KEYS: [&str; 3] = ["topic", "qos", "retain"];

/// Message keys that belong to another part of the format. Anything else on the
/// first line of a message is the payload, which is why this list is short.
const MQTT_MESSAGE_REFUSED: [&str; 4] = ["retained", "subscribe", "subprotocol", "client-id"];

fn known(key: &str, set: &[&str]) -> bool {
    set.iter().any(|k| same_key(k, key))
}

#[derive(Debug, Clone)]
struct PairSpan {
    name: Range<usize>,
    value: Range<usize>,
    line: Range<usize>,
}

#[derive(Debug, Clone)]
struct MessageSpans {
    name: String,
    header_end: usize,
    options: Vec<PairSpan>,
    payload: Option<Range<usize>>,
    end: usize,
}

#[derive(Debug, Clone)]
struct BlockSpans {
    name: String,
    name_span: Option<Range<usize>>,
    span: Range<usize>,
    url: String,
    url_span: Range<usize>,
    url_line_end: usize,
    options: Vec<PairSpan>,
    messages: Vec<MessageSpans>,
    messages_start: Option<usize>,
    pre: Option<Range<usize>>,
    pre_outer: Option<Range<usize>>,
    content_end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamBlock {
    pub name: String,
    pub index: usize,
    pub request: SavedRequest,
}

#[derive(Debug)]
pub struct StreamDoc {
    source: String,
    stem: String,
    flavor: Flavor,
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

fn read_url_line(
    source: &str,
    lines: &[Line<'_>],
    index: usize,
    flavor: Flavor,
) -> CoreResult<(String, Range<usize>, usize)> {
    let line = &lines[index];
    let raw = &source[line.start..line.end];
    let lead = raw.len() - raw.trim_start().len();
    let body = raw.trim();
    if body.is_empty() {
        return Err(text::parse_err(
            line.number,
            format_args!("{} needs a url on this line", flavor.noun()),
        ));
    }
    if body.contains(char::is_whitespace) {
        return Err(text::parse_err(
            line.number,
            format_args!(
                "the connection line is the url and nothing else, so it must not contain whitespace: {body:?}"
            ),
        ));
    }
    let start = line.start + lead;
    Ok((
        body.to_string(),
        start..start + body.len(),
        start + body.len(),
    ))
}

fn is_message_header(text: &str) -> bool {
    text.trim_start().starts_with(">>")
}

/// How a run of `key: value` lines is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Any valid name is accepted — a `.ws` connection line is an HTTP header.
    Any,
    /// Only the listed keys exist, and a line that is not one of them is a
    /// mistake rather than the start of something else.
    Closed,
    /// Only the listed keys are options; the first line that is not one is where
    /// the payload begins.
    Leading,
}

/// Reads `key: value` lines until something that is not one. `keys` is the set
/// this position accepts; `refused` is the set that fails loud instead of being
/// taken for a header or for payload.
#[allow(clippy::too_many_arguments)]
fn read_pairs(
    source: &str,
    lines: &[Line<'_>],
    from: usize,
    line_end: usize,
    keys: &[&str],
    refused: &[&str],
    mode: Mode,
    what: &str,
) -> CoreResult<(Vec<PairSpan>, usize, usize)> {
    let mut pairs: Vec<PairSpan> = Vec::new();
    let mut index = from;
    let mut end = line_end;
    while index < lines.len() {
        let line = &lines[index];
        let trimmed = line.text.trim_start();
        if trimmed.is_empty() || is_message_header(line.text) {
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
        let lead = raw.len() - raw.trim_start().len();
        let Some(colon) = raw.find(':') else {
            if mode == Mode::Leading {
                break;
            }
            return Err(text::parse_err(
                line.number,
                format_args!("expected `{what}`, found {:?}", line.text.trim()),
            ));
        };
        let name = raw[lead..colon].trim_end();
        if !text::valid_header_name(name) {
            if mode == Mode::Leading {
                break;
            }
            return Err(text::parse_err(
                line.number,
                format_args!("{name:?} is not a valid {what} name"),
            ));
        }
        if known(name, refused) {
            return Err(text::unsupported(
                line.number,
                format_args!(
                    "{name:?} reads like a Mándalo key but means nothing here — this line accepts {}",
                    match keys.is_empty() {
                        true => "no options at all".to_string(),
                        false => keys.join(", "),
                    }
                ),
            ));
        }
        if mode != Mode::Any && !known(name, keys) {
            if mode == Mode::Leading {
                break;
            }
            return Err(text::unsupported(
                line.number,
                format_args!(
                    "{name:?} is not one of the {what} keys — write one of {}",
                    keys.join(", ")
                ),
            ));
        }
        let after = &raw[colon + 1..];
        let value = after.trim();
        let value_start = line.start + colon + 1 + (after.len() - after.trim_start().len());
        pairs.push(PairSpan {
            name: line.start + lead..line.start + lead + name.len(),
            value: value_start..value_start + value.len(),
            line: line.start..line.end,
        });
        end = line.end;
        index += 1;
    }
    Ok((pairs, index, end))
}

fn read_payload(
    source: &str,
    lines: &[Line<'_>],
    from: usize,
    stop_at_message: bool,
) -> (Option<Range<usize>>, usize, usize) {
    let mut index = from;
    let mut start = None;
    let mut end = 0usize;
    while index < lines.len() {
        let line = &lines[index];
        if stop_at_message && is_message_header(line.text) {
            break;
        }
        let trimmed = line.text.trim();
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
    match start {
        Some(at) => {
            let trimmed = source[at..end].trim_end();
            if trimmed.is_empty() {
                (None, index, at)
            } else {
                (Some(at..at + trimmed.len()), index, at + trimmed.len())
            }
        }
        None => (None, index, 0),
    }
}

fn read_messages(
    source: &str,
    lines: &[Line<'_>],
    from: usize,
    flavor: Flavor,
) -> CoreResult<(Vec<MessageSpans>, Option<usize>, usize)> {
    let keys: &[&str] = match flavor {
        Flavor::Ws => &[],
        Flavor::Mqtt => &MESSAGE_KEYS,
    };
    let refused: &[&str] = match flavor {
        Flavor::Ws => &MESSAGE_KEYS,
        Flavor::Mqtt => &MQTT_MESSAGE_REFUSED,
    };
    let mut messages: Vec<MessageSpans> = Vec::new();
    let mut first = None;
    let mut content_end = 0usize;
    let mut index = from;
    while index < lines.len() {
        let line = &lines[index];
        if line.text.trim().is_empty() {
            index += 1;
            continue;
        }
        if text::is_comment(line.text) {
            index += 1;
            continue;
        }
        if !is_message_header(line.text) {
            if line.text.trim_start().starts_with('>') {
                return Err(text::unsupported(
                    line.number,
                    format_args!(
                        "{} has no single response, so a `> {{% … %}}` script has nothing to run against — read the events with `mandalo listen --json` instead",
                        flavor.noun()
                    ),
                ));
            }
            return Err(text::parse_err(
                line.number,
                format_args!(
                    "expected `>> name` to open a message, found {:?}",
                    line.text.trim()
                ),
            ));
        }
        let raw = &source[line.start..line.end];
        let lead = raw.len() - raw.trim_start().len();
        let after = lead + 2;
        let name = raw[after..].trim();
        if name.is_empty() {
            return Err(text::parse_err(
                line.number,
                "a message needs a name — write `>> subscribe`",
            ));
        }
        if messages.iter().any(|m| m.name == name) {
            return Err(CoreError::Conflict(format!(
                "line {}: two messages are named {name:?} — a message is addressed by name, so each one needs its own",
                line.number
            )));
        }
        first.get_or_insert(line.start);
        let (options, after_options, options_end) = read_pairs(
            source,
            lines,
            index + 1,
            line.end,
            keys,
            refused,
            Mode::Leading,
            "message option",
        )?;
        let (payload, next, payload_end) = read_payload(source, lines, after_options, true);
        let end = payload_end.max(options_end);
        content_end = content_end.max(end);
        messages.push(MessageSpans {
            name: name.to_string(),
            header_end: line.end,
            options,
            payload,
            end,
        });
        index = next;
    }
    Ok((messages, first, content_end))
}

fn parse_block(source: &str, segment: &Segment<'_>, flavor: Flavor) -> CoreResult<BlockSpans> {
    let lines = &segment.lines;
    let preamble = read_preamble(source, lines)?;
    if preamble.consumed >= lines.len() {
        let line = lines.first().map(|l| l.number).unwrap_or(1);
        return Err(text::parse_err(
            line,
            format_args!(
                "this block has no connection line — {} needs a url",
                flavor.noun()
            ),
        ));
    }
    let (url, url_span, url_line_end) = read_url_line(source, lines, preamble.consumed, flavor)?;

    let (keys, refused, mode, what): (&[&str], &[&str], Mode, &str) = match flavor {
        Flavor::Ws => (&WS_KEYS, &WS_REFUSED, Mode::Any, "Name: value"),
        Flavor::Mqtt => (&MQTT_KEYS, &[], Mode::Closed, "connection option"),
    };
    let (options, after_options, options_end) = read_pairs(
        source,
        lines,
        preamble.consumed + 1,
        url_line_end,
        keys,
        refused,
        mode,
        what,
    )?;
    for pair in &options {
        let key = &source[pair.name.clone()];
        let value = &source[pair.value.clone()];
        validate_option(flavor, key, value, text::line_of(lines, pair.name.start))?;
    }

    let (messages, messages_start, messages_end) =
        read_messages(source, lines, after_options, flavor)?;
    for message in &messages {
        for pair in &message.options {
            let key = &source[pair.name.clone()];
            let value = &source[pair.value.clone()];
            validate_message_option(key, value, text::line_of(lines, pair.name.start))?;
        }
    }

    let name = segment
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .or(preamble.named)
        .unwrap_or_else(|| url.clone());

    Ok(BlockSpans {
        name,
        name_span: segment.name_span.clone(),
        span: segment.span.clone(),
        url,
        url_span,
        url_line_end,
        options,
        messages,
        messages_start,
        content_end: messages_end.max(options_end),
        pre: preamble.pre,
        pre_outer: preamble.pre_outer,
    })
}

fn parse_bool(line: usize, key: &str, value: &str) -> CoreResult<bool> {
    match value.trim() {
        "true" | "on" | "yes" => Ok(true),
        "false" | "off" | "no" => Ok(false),
        other => Err(text::parse_err(
            line,
            format_args!("`{key}` is true or false, not {other:?}"),
        )),
    }
}

fn parse_u64(line: usize, key: &str, value: &str) -> CoreResult<u64> {
    value.trim().parse::<u64>().map_err(|_| {
        text::parse_err(
            line,
            format_args!(
                "`{key}` is a whole number of seconds, not {:?}",
                value.trim()
            ),
        )
    })
}

fn parse_qos(line: usize, value: &str) -> CoreResult<u8> {
    match value.trim() {
        "0" => Ok(0),
        "1" => Ok(1),
        "2" => Ok(2),
        other => Err(text::parse_err(
            line,
            format_args!("mqtt has qos 0, 1 and 2 — {other:?} is not one of them"),
        )),
    }
}

/// `sensors/#; qos=1`. The `; key=value` parameter is the same shape a form-data
/// file line uses, so the format has one spelling for "this line, plus a detail".
fn parse_subscription(line: usize, value: &str) -> CoreResult<Subscription> {
    let (topic, qos) = match value.split_once(';') {
        Some((topic, params)) => {
            let Some((key, level)) = params.split_once('=') else {
                return Err(text::parse_err(
                    line,
                    "a subscription takes one parameter, written `; qos=1`",
                ));
            };
            if !key.trim().eq_ignore_ascii_case("qos") {
                return Err(text::unsupported(
                    line,
                    format_args!("a subscription takes only `; qos=…`, not {:?}", key.trim()),
                ));
            }
            (topic, parse_qos(line, level)?)
        }
        None => (value, 0),
    };
    let topic = topic.trim();
    if topic.is_empty() {
        return Err(text::parse_err(line, "`subscribe` needs a topic filter"));
    }
    Ok(Subscription {
        topic: topic.to_string(),
        qos,
    })
}

fn validate_option(flavor: Flavor, key: &str, value: &str, line: usize) -> CoreResult<()> {
    match flavor {
        Flavor::Ws => {
            if same_key(key, "auto-reconnect") {
                parse_bool(line, key, value)?;
            } else if same_key(key, "ping-interval") {
                parse_u64(line, key, value)?;
            } else if same_key(key, "subprotocol") && value.trim().is_empty() {
                return Err(text::parse_err(line, "`subprotocol` needs a name"));
            }
        }
        Flavor::Mqtt => {
            if same_key(key, "keep-alive") {
                parse_u64(line, key, value)?;
            } else if same_key(key, "clean-session") {
                parse_bool(line, key, value)?;
            } else if same_key(key, "subscribe") {
                parse_subscription(line, value)?;
            } else if same_key(key, "protocol-version") {
                match value.trim() {
                    "3.1.1" | "5" => {}
                    other => {
                        return Err(text::parse_err(
                            line,
                            format_args!("`protocol-version` is 3.1.1 or 5, not {other:?}"),
                        ))
                    }
                }
            } else if same_key(key, "password") {
                collection::reject_literal_password(value).map_err(|e| {
                    text::unsupported(line, e.to_string().trim_start_matches("line "))
                })?;
            } else if same_key(key, "client-id") && value.trim().is_empty() {
                return Err(text::parse_err(line, "`client-id` needs a value"));
            }
        }
    }
    Ok(())
}

fn validate_message_option(key: &str, value: &str, line: usize) -> CoreResult<()> {
    if same_key(key, "qos") {
        parse_qos(line, value)?;
    } else if same_key(key, "retain") {
        parse_bool(line, key, value)?;
    } else if same_key(key, "topic") && value.trim().is_empty() {
        return Err(text::parse_err(line, "`topic` needs a value"));
    }
    Ok(())
}

impl StreamDoc {
    pub fn parse(flavor: Flavor, stem: &str, source: &str) -> CoreResult<StreamDoc> {
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
            blocks.push(parse_block(source, segment, flavor)?);
        }
        Ok(StreamDoc {
            source: source.to_string(),
            stem: slug(stem),
            flavor,
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

    fn pair(&self, span: &PairSpan) -> (String, String) {
        (
            self.source[span.name.clone()].to_string(),
            self.source[span.value.clone()].to_string(),
        )
    }

    pub fn raw(&self, index: usize) -> CoreResult<StreamBlock> {
        let block = self.block(index)?;
        let id = format!("{}-{index}", self.stem);
        let mut headers: Vec<(String, String)> = Vec::new();
        let mut stream = SavedStream::default();
        for span in &block.options {
            let (name, value) = self.pair(span);
            match self.flavor {
                Flavor::Ws if same_key(&name, "subprotocol") => stream.subprotocols.push(value),
                Flavor::Ws if same_key(&name, "auto-reconnect") => {
                    stream.auto_reconnect = Some(parse_bool(0, &name, &value)?)
                }
                Flavor::Ws if same_key(&name, "ping-interval") => {
                    stream.ping_interval_ms = Some(parse_u64(0, &name, &value)? * 1000)
                }
                Flavor::Ws => headers.push((name, value)),
                Flavor::Mqtt if same_key(&name, "client-id") => stream.client_id = Some(value),
                Flavor::Mqtt if same_key(&name, "username") => stream.username = Some(value),
                Flavor::Mqtt if same_key(&name, "password") => stream.password = Some(value),
                Flavor::Mqtt if same_key(&name, "keep-alive") => {
                    stream.keep_alive_secs = Some(parse_u64(0, &name, &value)?)
                }
                Flavor::Mqtt if same_key(&name, "clean-session") => {
                    stream.clean_session = Some(parse_bool(0, &name, &value)?)
                }
                Flavor::Mqtt if same_key(&name, "protocol-version") => {
                    stream.protocol_version = Some(match value.trim() {
                        "5" => MqttVersion::V5,
                        _ => MqttVersion::V311,
                    })
                }
                Flavor::Mqtt => stream.subscriptions.push(parse_subscription(0, &value)?),
            }
        }

        for (at, span) in block.messages.iter().enumerate() {
            let payload = span
                .payload
                .clone()
                .map(|s| self.source[s].to_string())
                .unwrap_or_default();
            let message = match self.flavor {
                Flavor::Ws => Outgoing::text(payload),
                Flavor::Mqtt => {
                    let mut topic = None;
                    let mut qos = 0u8;
                    let mut retain = false;
                    for option in &span.options {
                        let (name, value) = self.pair(option);
                        if same_key(&name, "topic") {
                            topic = Some(value);
                        } else if same_key(&name, "qos") {
                            qos = parse_qos(0, &value)?;
                        } else if same_key(&name, "retain") {
                            retain = parse_bool(0, &name, &value)?;
                        }
                    }
                    Outgoing::Publish {
                        topic: topic.ok_or_else(|| {
                            CoreError::Parse(format!(
                                "the message {:?} has no `topic:` line, and an mqtt message publishes to a topic",
                                span.name
                            ))
                        })?,
                        payload,
                        qos,
                        retain,
                    }
                }
            };
            stream.messages.push(SavedMessage {
                id: format!("{id}-{at}"),
                name: span.name.clone(),
                message,
            });
        }

        Ok(StreamBlock {
            name: block.name.clone(),
            index,
            request: SavedRequest {
                id,
                name: block.name.clone(),
                kind: self.flavor.kind().to_string(),
                method: self.flavor.method().to_string(),
                url: block.url.clone(),
                description: None,
                headers,
                auth: Auth::None,
                body: Body::None,
                grpc: None,
                stream: Some(stream),
                scripts: Scripts {
                    pre: block.pre.clone().map(|s| text::dedent(&self.source[s])),
                    post: None,
                },
                tests: Vec::new(),
                captures: Vec::new(),
            },
        })
    }

    pub fn resolved(&self, index: usize) -> CoreResult<StreamBlock> {
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
        if let Some(stream) = request.stream.as_mut() {
            let one = |value: &Option<String>| value.as_ref().map(|v| text::substitute(v, &vars));
            stream.subprotocols = stream
                .subprotocols
                .iter()
                .map(|p| text::substitute(p, &vars))
                .collect();
            stream.client_id = one(&stream.client_id);
            stream.username = one(&stream.username);
            stream.password = one(&stream.password);
            for subscription in &mut stream.subscriptions {
                subscription.topic = text::substitute(&subscription.topic, &vars);
            }
            for saved in &mut stream.messages {
                saved.message = substitute_outgoing(&saved.message, &vars);
            }
        }
        Ok(block)
    }

    /// The connection lines as they should sit in the file, each keeping the
    /// spelling the file already chose for it.
    fn wire_options(
        &self,
        block: &BlockSpans,
        request: &SavedRequest,
    ) -> CoreResult<Vec<(String, String)>> {
        let stream = stream_of(request, self.flavor)?;
        let mut out: Vec<(String, String)> = Vec::new();
        match self.flavor {
            Flavor::Ws => {
                for protocol in &stream.subprotocols {
                    out.push(("subprotocol".to_string(), protocol.clone()));
                }
                if let Some(on) = stream.auto_reconnect {
                    out.push(("auto-reconnect".to_string(), on.to_string()));
                }
                if let Some(millis) = stream.ping_interval_ms {
                    out.push(("ping-interval".to_string(), (millis / 1000).to_string()));
                }
                out.extend(request.headers.iter().cloned());
            }
            Flavor::Mqtt => {
                if !request.headers.is_empty() {
                    return Err(CoreError::Unsupported(MQTT_HAS_NO_HEADERS.to_string()));
                }
                if let Some(id) = &stream.client_id {
                    out.push(("client-id".to_string(), id.clone()));
                }
                if let Some(user) = &stream.username {
                    out.push(("username".to_string(), user.clone()));
                }
                if let Some(password) = &stream.password {
                    collection::reject_literal_password(password)?;
                    out.push(("password".to_string(), password.clone()));
                }
                if let Some(seconds) = stream.keep_alive_secs {
                    out.push(("keep-alive".to_string(), seconds.to_string()));
                }
                if let Some(clean) = stream.clean_session {
                    out.push(("clean-session".to_string(), clean.to_string()));
                }
                if let Some(version) = stream.protocol_version {
                    out.push(("protocol-version".to_string(), version_label(version)));
                }
                for subscription in &stream.subscriptions {
                    out.push(("subscribe".to_string(), render_subscription(subscription)));
                }
            }
        }
        Ok(keep_spelling(&self.written_options(block), out))
    }

    fn written_options(&self, block: &BlockSpans) -> Vec<(String, String)> {
        block.options.iter().map(|p| self.pair(p)).collect()
    }

    pub fn replace(&self, index: usize, request: &SavedRequest) -> CoreResult<String> {
        let block = self.block(index)?;
        let current = self.raw(index)?;
        let nl = self.newline;
        if request.kind != self.flavor.kind() {
            return Err(CoreError::Unsupported(format!(
                "a .{} file holds {} requests, not {:?}",
                self.flavor.extension(),
                self.flavor.kind(),
                request.kind
            )));
        }
        text::reject_inexpressible(request, self.flavor.extension())?;
        text::reject_description_edit(request, self.flavor.extension())?;
        reject_post_script(request, self.flavor)?;
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
        if request.url != current.request.url {
            edits.push((block.url_span.clone(), request.url.clone()));
        }

        let desired = self.wire_options(block, request)?;
        let written = self.written_options(block);
        if desired != written {
            edits.extend(pair_edits(
                nl,
                &block.options,
                block.url_line_end,
                &written,
                &desired,
            ));
        }

        edits.extend(self.message_edits(block, &current, request)?);
        edits.extend(self.script_edits(block, &current.request.scripts.pre, &request.scripts.pre));

        Ok(text::splice(&self.source, edits))
    }

    fn message_edits(
        &self,
        block: &BlockSpans,
        current: &StreamBlock,
        request: &SavedRequest,
    ) -> CoreResult<Vec<(Range<usize>, String)>> {
        let nl = self.newline;
        let wanted = stream_of(request, self.flavor)?;
        let held = stream_of(&current.request, self.flavor)?;
        if wanted.messages == held.messages {
            return Ok(Vec::new());
        }
        let aligned = wanted.messages.len() == held.messages.len()
            && wanted
                .messages
                .iter()
                .zip(&held.messages)
                .all(|(a, b)| a.name == b.name);
        if !aligned {
            let rendered = render_messages(&wanted.messages, self.flavor, nl)?;
            return Ok(match block.messages_start {
                Some(start) => vec![(start..block.content_end.max(start), rendered)],
                None if rendered.is_empty() => Vec::new(),
                None => vec![(
                    block.content_end..block.content_end,
                    format!("{nl}{nl}{rendered}"),
                )],
            });
        }
        let mut edits = Vec::new();
        for (span, message) in block.messages.iter().zip(&wanted.messages) {
            let written: Vec<(String, String)> =
                span.options.iter().map(|p| self.pair(p)).collect();
            let desired = keep_spelling(&written, message_options(&message.message, self.flavor)?);
            if desired != written {
                edits.extend(pair_edits(
                    nl,
                    &span.options,
                    span.header_end,
                    &written,
                    &desired,
                ));
            }
            let payload = payload_of(&message.message, self.flavor)?;
            let payload = payload.trim();
            let currently = span
                .payload
                .clone()
                .map(|s| self.source[s].to_string())
                .unwrap_or_default();
            if payload != currently.trim() {
                edits.push(match &span.payload {
                    Some(existing) => (existing.clone(), payload.to_string()),
                    None => (span.end..span.end, format!("{nl}{nl}{payload}")),
                });
            }
        }
        Ok(edits)
    }

    fn script_edits(
        &self,
        block: &BlockSpans,
        current: &Option<String>,
        desired: &Option<String>,
    ) -> Vec<(Range<usize>, String)> {
        let normalized = desired.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if current.as_deref() == normalized {
            return Vec::new();
        }
        let nl = self.newline;
        match (&block.pre, &block.pre_outer, normalized) {
            (Some(span), _, Some(text)) => vec![(span.clone(), format!("{nl}{text}{nl}"))],
            (_, Some(span), None) => vec![(span.clone(), String::new())],
            (None, None, Some(text)) => vec![(
                block.span.start..block.span.start,
                format!("< {{%{nl}{text}{nl}%}}{nl}"),
            )],
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
        let rendered = render_request(request, self.flavor, nl)?;
        let trimmed = self.source.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Ok(rendered);
        }
        Ok(format!("{trimmed}{nl}{nl}{rendered}"))
    }
}

fn stream_of(request: &SavedRequest, flavor: Flavor) -> CoreResult<&SavedStream> {
    request.stream.as_ref().ok_or_else(|| {
        CoreError::Unsupported(format!(
            "a .{} file holds {} requests — this one carries no connection",
            flavor.extension(),
            flavor.kind()
        ))
    })
}

fn reject_post_script(request: &SavedRequest, flavor: Flavor) -> CoreResult<()> {
    if request
        .scripts
        .post
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return Err(CoreError::Unsupported(format!(
            "{} has no single response, so a `> {{% … %}}` script has nothing to run against — read the events with `mandalo listen --json` instead",
            flavor.noun()
        )));
    }
    Ok(())
}

const MQTT_HAS_NO_HEADERS: &str =
    "an mqtt connection carries no headers — it signs in with client-id, username and password";

fn version_label(version: MqttVersion) -> String {
    match version {
        MqttVersion::V311 => "3.1.1".to_string(),
        MqttVersion::V5 => "5".to_string(),
    }
}

fn substitute_outgoing(
    message: &Outgoing,
    vars: &std::collections::BTreeMap<String, String>,
) -> Outgoing {
    match message {
        Outgoing::Text { text } => Outgoing::text(text::substitute(text, vars)),
        Outgoing::Publish {
            topic,
            payload,
            qos,
            retain,
        } => Outgoing::Publish {
            topic: text::substitute(topic, vars),
            payload: text::substitute(payload, vars),
            qos: *qos,
            retain: *retain,
        },
        other => other.clone(),
    }
}

/// What a message the format can write actually holds. Everything a socket can
/// send that is not one of these two is a live action or a byte blob, and a text
/// file has no honest line for it.
fn payload_of(message: &Outgoing, flavor: Flavor) -> CoreResult<&str> {
    match (message, flavor) {
        (Outgoing::Text { text }, Flavor::Ws) => Ok(text),
        (Outgoing::Publish { payload, .. }, Flavor::Mqtt) => Ok(payload),
        _ => Err(CoreError::Unsupported(format!(
            "a .{} file writes {}, and this message is not one — send it from the connection instead",
            flavor.extension(),
            match flavor {
                Flavor::Ws => "text messages",
                Flavor::Mqtt => "publishes",
            }
        ))),
    }
}

fn render_subscription(subscription: &Subscription) -> String {
    match subscription.qos {
        0 => subscription.topic.clone(),
        qos => format!("{}; qos={qos}", subscription.topic),
    }
}

fn message_options(message: &Outgoing, flavor: Flavor) -> CoreResult<Vec<(String, String)>> {
    payload_of(message, flavor)?;
    let mut out = Vec::new();
    if let Outgoing::Publish {
        topic, qos, retain, ..
    } = message
    {
        out.push(("topic".to_string(), topic.clone()));
        if *qos != 0 {
            out.push(("qos".to_string(), qos.to_string()));
        }
        if *retain {
            out.push(("retain".to_string(), retain.to_string()));
        }
    }
    Ok(out)
}

/// Rewrites each desired key with the spelling the file already used for it, so
/// a file written with `clientId` is not silently reformatted to `client-id`.
fn keep_spelling(
    written: &[(String, String)],
    desired: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let mut taken = vec![false; written.len()];
    desired
        .into_iter()
        .map(|(key, value)| {
            let found = written
                .iter()
                .enumerate()
                .find(|(at, (k, _))| !taken[*at] && same_key(k, &key));
            match found {
                Some((at, (k, _))) => {
                    taken[at] = true;
                    (k.clone(), value)
                }
                None => (key, value),
            }
        })
        .collect()
}

/// The same in-place edit the header lines of a `.http` file get: a changed value
/// stays on its own line, and the comments between the lines survive.
fn pair_edits(
    nl: &str,
    spans: &[PairSpan],
    before: usize,
    written: &[(String, String)],
    desired: &[(String, String)],
) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    let mut taken = vec![false; desired.len()];
    let mut last_line_end = before;
    for (position, (name, value)) in written.iter().enumerate() {
        let span = &spans[position];
        let matched = desired
            .iter()
            .enumerate()
            .find(|(at, (k, _))| !taken[*at] && same_key(k, name));
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

fn render_messages(messages: &[SavedMessage], flavor: Flavor, nl: &str) -> CoreResult<String> {
    let mut out = String::new();
    for saved in messages {
        if saved.name.trim().is_empty() {
            return Err(CoreError::InvalidName(
                "every message needs a name".to_string(),
            ));
        }
        out.push_str(&format!("{nl}>> {}{nl}", saved.name.trim()));
        for (key, value) in message_options(&saved.message, flavor)? {
            out.push_str(&format!("{key}: {value}{nl}"));
        }
        let payload = payload_of(&saved.message, flavor)?.trim();
        if !payload.is_empty() {
            out.push_str(nl);
            for line in payload.split('\n') {
                out.push_str(line.trim_end_matches('\r'));
                out.push_str(nl);
            }
        }
    }
    Ok(out.trim_matches(['\r', '\n']).to_string())
}

pub fn render_request(request: &SavedRequest, flavor: Flavor, nl: &str) -> CoreResult<String> {
    if request.kind != flavor.kind() {
        return Err(CoreError::Unsupported(format!(
            "a .{} file holds {} requests, not {:?}",
            flavor.extension(),
            flavor.kind(),
            request.kind
        )));
    }
    let stream = stream_of(request, flavor)?;
    text::reject_inexpressible(request, flavor.extension())?;
    reject_post_script(request, flavor)?;
    if flavor == Flavor::Mqtt && !request.headers.is_empty() {
        return Err(CoreError::Unsupported(MQTT_HAS_NO_HEADERS.to_string()));
    }
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
    out.push_str(&format!("{}{nl}", request.url));
    match flavor {
        Flavor::Ws => {
            for protocol in &stream.subprotocols {
                out.push_str(&format!("subprotocol: {protocol}{nl}"));
            }
            if let Some(on) = stream.auto_reconnect {
                out.push_str(&format!("auto-reconnect: {on}{nl}"));
            }
            if let Some(millis) = stream.ping_interval_ms {
                out.push_str(&format!("ping-interval: {}{nl}", millis / 1000));
            }
            for (name, value) in &request.headers {
                out.push_str(&format!("{name}: {value}{nl}"));
            }
        }
        Flavor::Mqtt => {
            if let Some(id) = &stream.client_id {
                out.push_str(&format!("client-id: {id}{nl}"));
            }
            if let Some(user) = &stream.username {
                out.push_str(&format!("username: {user}{nl}"));
            }
            if let Some(password) = &stream.password {
                collection::reject_literal_password(password)?;
                out.push_str(&format!("password: {password}{nl}"));
            }
            if let Some(seconds) = stream.keep_alive_secs {
                out.push_str(&format!("keep-alive: {seconds}{nl}"));
            }
            if let Some(clean) = stream.clean_session {
                out.push_str(&format!("clean-session: {clean}{nl}"));
            }
            if let Some(version) = stream.protocol_version {
                out.push_str(&format!("protocol-version: {}{nl}", version_label(version)));
            }
            for subscription in &stream.subscriptions {
                out.push_str(&format!(
                    "subscribe: {}{nl}",
                    render_subscription(subscription)
                ));
            }
        }
    }
    let messages = render_messages(&stream.messages, flavor, nl)?;
    if !messages.is_empty() {
        out.push_str(nl);
        out.push_str(&messages);
        out.push_str(nl);
    }
    Ok(out)
}

pub fn render_file(requests: &[SavedRequest], flavor: Flavor, nl: &str) -> CoreResult<String> {
    let mut parts = Vec::with_capacity(requests.len());
    for request in requests {
        parts.push(render_request(request, flavor, nl)?);
    }
    Ok(parts.join(nl))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: &str = "### Chat socket\nwss://{{host}}/socket\nsubprotocol: chat.v2\nauthorization: Bearer {{token}}\n\n>> subscribe\n{\"op\": \"sub\", \"channel\": \"prices\"}\n\n>> ping\n{\"op\": \"ping\"}\n";

    const SENSORS: &str = "### Sensors\nmqtt://{{broker}}:1883\nclient-id: mandalo-probe\nusername: {{user}}\nkeep-alive: 30\nsubscribe: sensors/#; qos=1\n\n>> report\ntopic: sensors/{{room}}/temp\nqos: 1\n\n{\"c\": 21.5}\n";

    fn ws(source: &str) -> StreamDoc {
        StreamDoc::parse(Flavor::Ws, "chat.ws", source).expect("the file parses")
    }

    fn mqtt(source: &str) -> StreamDoc {
        StreamDoc::parse(Flavor::Mqtt, "sensors.mqtt", source).expect("the file parses")
    }

    #[test]
    fn a_websocket_block_reads_url_headers_and_named_messages() {
        let doc = ws(CHAT);
        let request = doc.raw(0).unwrap().request;
        assert_eq!(request.kind, "websocket");
        assert_eq!(request.method, "WS");
        assert_eq!(request.url, "wss://{{host}}/socket");
        assert_eq!(
            request.headers,
            vec![("authorization".to_string(), "Bearer {{token}}".to_string())]
        );
        let stream = request.stream.unwrap();
        assert_eq!(stream.subprotocols, vec!["chat.v2".to_string()]);
        assert_eq!(stream.auto_reconnect, None);
        assert_eq!(
            stream
                .messages
                .iter()
                .map(|m| (m.id.as_str(), m.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("chat-ws-0-0", "subscribe"), ("chat-ws-0-1", "ping")]
        );
        assert_eq!(
            stream.messages[1].message,
            Outgoing::text("{\"op\": \"ping\"}")
        );
    }

    #[test]
    fn an_mqtt_block_reads_options_subscriptions_and_message_topics() {
        let doc = mqtt(SENSORS);
        let request = doc.raw(0).unwrap().request;
        assert_eq!(request.kind, "mqtt");
        assert_eq!(request.method, "MQTT");
        assert!(request.headers.is_empty());
        let stream = request.stream.unwrap();
        assert_eq!(stream.client_id.as_deref(), Some("mandalo-probe"));
        assert_eq!(stream.username.as_deref(), Some("{{user}}"));
        assert_eq!(stream.keep_alive_secs, Some(30));
        assert_eq!(stream.clean_session, None);
        assert_eq!(stream.subscriptions[0].topic, "sensors/#");
        assert_eq!(stream.subscriptions[0].qos, 1);
        assert_eq!(
            stream.messages[0].message,
            Outgoing::Publish {
                topic: "sensors/{{room}}/temp".to_string(),
                payload: "{\"c\": 21.5}".to_string(),
                qos: 1,
                retain: false,
            }
        );
    }

    #[test]
    fn saving_an_untouched_request_returns_the_file_byte_for_byte() {
        for (doc, source) in [(ws(CHAT), CHAT), (mqtt(SENSORS), SENSORS)] {
            let request = doc.raw(0).unwrap().request;
            assert_eq!(doc.replace(0, &request).unwrap(), source);
        }
    }

    #[test]
    fn rendering_a_parsed_request_reads_back_the_same() {
        for (doc, flavor) in [(ws(CHAT), Flavor::Ws), (mqtt(SENSORS), Flavor::Mqtt)] {
            let request = doc.raw(0).unwrap().request;
            let rendered = render_request(&request, flavor, "\n").unwrap();
            let again = StreamDoc::parse(flavor, "again", &rendered).unwrap();
            let read = again.raw(0).unwrap().request;
            assert_eq!(read.url, request.url);
            assert_eq!(read.headers, request.headers);
            let (a, b) = (read.stream.unwrap(), request.stream.unwrap());
            assert_eq!(a.subscriptions, b.subscriptions);
            assert_eq!(a.subprotocols, b.subprotocols);
            assert_eq!(a.keep_alive_secs, b.keep_alive_secs);
            assert_eq!(
                a.messages.iter().map(|m| &m.message).collect::<Vec<_>>(),
                b.messages.iter().map(|m| &m.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn both_spellings_of_every_option_parse_the_same() {
        let kebab = mqtt("### A\nmqtt://b.dev\nclient-id: probe\nkeep-alive: 15\nclean-session: false\nprotocol-version: 3.1.1\nsubscribe: a/#\n");
        let camel = mqtt("### A\nmqtt://b.dev\nclientId: probe\nkeepAlive: 15\ncleanSession: false\nprotocolVersion: 3.1.1\nsubscriptions: a/#\n");
        assert_eq!(
            kebab.raw(0).unwrap().request.stream,
            camel.raw(0).unwrap().request.stream
        );

        let kebab =
            ws("### A\nws://x.dev\nsubprotocol: chat\nauto-reconnect: true\nping-interval: 20\n");
        let camel =
            ws("### A\nws://x.dev\nsubprotocols: chat\nautoReconnect: on\npingInterval: 20\n");
        assert_eq!(
            kebab.raw(0).unwrap().request.stream,
            camel.raw(0).unwrap().request.stream
        );
    }

    #[test]
    fn a_camel_cased_key_keeps_its_spelling_when_the_value_changes() {
        let source = "### A\nmqtt://b.dev\nkeepAlive: 15\n";
        let doc = mqtt(source);
        let mut request = doc.raw(0).unwrap().request;
        request.stream.as_mut().unwrap().keep_alive_secs = Some(45);
        assert_eq!(
            doc.replace(0, &request).unwrap(),
            "### A\nmqtt://b.dev\nkeepAlive: 45\n"
        );
    }

    #[test]
    fn an_option_the_file_never_wrote_stays_unwritten() {
        let source = "### A\nws://x.dev\n";
        let doc = ws(source);
        let request = doc.raw(0).unwrap().request;
        assert_eq!(request.stream.as_ref().unwrap().auto_reconnect, None);
        assert_eq!(doc.replace(0, &request).unwrap(), source);
        assert_eq!(render_request(&request, Flavor::Ws, "\n").unwrap(), source);
    }

    #[test]
    fn an_unknown_reserved_looking_key_is_refused() {
        let err = StreamDoc::parse(Flavor::Ws, "a.ws", "### A\nws://x.dev\nclient-id: nope\n")
            .unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        assert!(err.to_string().contains("client-id"), "{err}");

        let err = StreamDoc::parse(Flavor::Mqtt, "a.mqtt", "### A\nmqtt://b.dev\nx-trace: 1\n")
            .unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");

        let err = StreamDoc::parse(
            Flavor::Ws,
            "a.ws",
            "### A\nws://x.dev\n\n>> go\ntopic: a/b\n",
        )
        .unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
    }

    #[test]
    fn a_literal_password_is_refused_on_the_way_in_and_on_the_way_out() {
        let err = StreamDoc::parse(
            Flavor::Mqtt,
            "a.mqtt",
            "### A\nmqtt://b.dev\npassword: hunter2\n",
        )
        .unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        assert!(err.to_string().contains("secret = true"), "{err}");

        let doc = mqtt("### A\nmqtt://b.dev\npassword: {{brokerPassword}}\n");
        let mut request = doc.raw(0).unwrap().request;
        assert_eq!(
            request.stream.as_ref().unwrap().password.as_deref(),
            Some("{{brokerPassword}}")
        );
        request.stream.as_mut().unwrap().password = Some("hunter2".to_string());
        assert_eq!(
            doc.replace(0, &request).unwrap_err().code(),
            "E_UNSUPPORTED"
        );
        assert_eq!(
            render_request(&request, Flavor::Mqtt, "\n")
                .unwrap_err()
                .code(),
            "E_UNSUPPORTED"
        );
    }

    #[test]
    fn a_message_the_format_cannot_write_is_refused_rather_than_dropped() {
        let doc = ws(CHAT);
        let mut request = doc.raw(0).unwrap().request;
        request.stream.as_mut().unwrap().messages[0].message = Outgoing::Ping;
        let err = doc.replace(0, &request).unwrap_err();
        assert_eq!(err.code(), "E_UNSUPPORTED");
        assert!(err.to_string().contains("text messages"), "{err}");
    }

    #[test]
    fn an_mqtt_message_without_a_topic_is_refused_because_a_publish_needs_one() {
        let err = StreamDoc::parse(Flavor::Mqtt, "a.mqtt", "### A\nmqtt://b.dev\n\n>> go\n1\n")
            .expect("the file parses")
            .raw(0)
            .unwrap_err();
        assert!(err.to_string().contains("publishes to a topic"), "{err}");
    }

    #[test]
    fn a_response_script_is_refused_because_a_stream_has_no_one_response() {
        let err = StreamDoc::parse(
            Flavor::Ws,
            "a.ws",
            "### A\nws://x.dev\n\n> {%\npm.test('x', function () {});\n%}\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("no single response"), "{err}");
    }

    #[test]
    fn a_pre_connect_script_survives_the_round_trip() {
        let source = "### A\n< {%\npm.environment.set('t', '1');\n%}\nws://x.dev\n";
        let doc = ws(source);
        let request = doc.raw(0).unwrap().request;
        assert_eq!(
            request.scripts.pre.as_deref(),
            Some("pm.environment.set('t', '1');")
        );
        assert_eq!(doc.replace(0, &request).unwrap(), source);
    }

    #[test]
    fn two_messages_may_not_share_a_name() {
        let err = StreamDoc::parse(
            Flavor::Ws,
            "a.ws",
            "### A\nws://x.dev\n\n>> go\n1\n\n>> go\n2\n",
        )
        .unwrap_err();
        assert_eq!(err.code(), "E_CONFLICT");
    }

    #[test]
    fn a_bad_option_value_names_the_key_and_the_line() {
        for (flavor, source, needle) in [
            (
                Flavor::Mqtt,
                "### A\nmqtt://b.dev\nkeep-alive: soon\n",
                "whole number",
            ),
            (
                Flavor::Mqtt,
                "### A\nmqtt://b.dev\nsubscribe: a/b; qos=9\n",
                "qos 0, 1 and 2",
            ),
            (
                Flavor::Ws,
                "### A\nws://x.dev\nauto-reconnect: maybe\n",
                "true or false",
            ),
        ] {
            let err = StreamDoc::parse(flavor, "a", source).unwrap_err();
            assert!(err.to_string().contains(needle), "{err}");
        }
    }

    #[test]
    fn vars_declared_in_the_file_resolve_before_the_request_leaves() {
        let doc =
            mqtt("@room = kitchen\n\n### A\nmqtt://b.dev\n\n>> t\ntopic: sensors/{{room}}\n\n1\n");
        let stream = doc.resolved(0).unwrap().request.stream.unwrap();
        match &stream.messages[0].message {
            Outgoing::Publish { topic, .. } => assert_eq!(topic, "sensors/kitchen"),
            other => panic!("expected a publish, got {other:?}"),
        }
    }

    #[test]
    fn a_block_with_no_url_says_so() {
        let err = StreamDoc::parse(Flavor::Ws, "a.ws", "### A\n# nothing here\n").unwrap_err();
        assert!(err.to_string().contains("no connection line"), "{err}");
    }
}
