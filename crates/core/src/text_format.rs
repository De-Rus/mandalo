use crate::collection::SavedRequest;
use crate::error::{CoreError, CoreResult};
use std::collections::BTreeMap;
use std::ops::Range;

/// One source line, with the byte offsets an edit needs. `text` excludes the line
/// terminator, so a CRLF file and an LF file parse to the same tokens while the
/// spans still point at the real bytes.
pub struct Line<'a> {
    pub text: &'a str,
    pub start: usize,
    pub end: usize,
    pub number: usize,
}

pub fn lines(source: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut number = 1usize;
    while start < source.len() {
        let end = source[start..]
            .find('\n')
            .map(|at| start + at)
            .unwrap_or(source.len());
        let mut text_end = end;
        if source[start..end].ends_with('\r') {
            text_end -= 1;
        }
        out.push(Line {
            text: &source[start..text_end],
            start,
            end: text_end,
            number,
        });
        start = end + 1;
        number += 1;
    }
    out
}

pub fn newline_of(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

pub fn parse_err(line: usize, message: impl std::fmt::Display) -> CoreError {
    CoreError::Parse(format!("line {line}: {message}"))
}

pub fn unsupported(line: usize, message: impl std::fmt::Display) -> CoreError {
    CoreError::Unsupported(format!("line {line}: {message}"))
}

pub fn is_separator(text: &str) -> bool {
    text.starts_with("###")
}

pub fn is_comment(text: &str) -> bool {
    let trimmed = text.trim_start();
    (trimmed.starts_with('#') && !is_separator(trimmed)) || trimmed.starts_with("//")
}

pub fn comment_body(text: &str) -> &str {
    let trimmed = text.trim_start();
    trimmed
        .strip_prefix("//")
        .or_else(|| trimmed.strip_prefix('#'))
        .unwrap_or(trimmed)
        .trim()
}

pub fn var_definition(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start().strip_prefix('@')?;
    let (name, value) = rest.split_once('=')?;
    Some((name.trim(), value.trim()))
}

pub fn valid_var_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

pub fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '{' | '}' | '$'))
}

/// A file a request body or a proto reference points at. Workspace-relative only:
/// a collection is shared, untrusted configuration, so an absolute path or a `..`
/// escape has to be a hard stop rather than a convenience.
pub fn workspace_relative(line: usize, raw: &str, what: &str) -> CoreResult<String> {
    let cleaned = raw.trim();
    if cleaned.is_empty() {
        return Err(parse_err(line, format!("{what} needs a path")));
    }
    if cleaned.starts_with('/') || cleaned.starts_with('\\') || cleaned.contains(':') {
        return Err(CoreError::PathEscape(format!(
            "line {line}: {what} must be a workspace-relative path, not {cleaned:?}"
        )));
    }
    let normalized = cleaned.trim_start_matches("./").to_string();
    if normalized
        .split(['/', '\\'])
        .any(|part| part == ".." || part.is_empty())
    {
        return Err(CoreError::PathEscape(format!(
            "line {line}: {what} must stay inside the workspace: {cleaned:?}"
        )));
    }
    Ok(normalized)
}

pub struct Segment<'a> {
    pub lines: Vec<Line<'a>>,
    pub name: Option<String>,
    pub name_span: Option<Range<usize>>,
    /// From the first byte of the `###` line (or of the file) to the last byte of
    /// the segment, so a whole request can be cut out of the file.
    pub span: Range<usize>,
}

pub fn segments<'a>(source: &'a str, all: Vec<Line<'a>>) -> Vec<Segment<'a>> {
    let mut out: Vec<Segment<'a>> = vec![Segment {
        lines: Vec::new(),
        name: None,
        name_span: None,
        span: 0..0,
    }];
    for line in all {
        if is_separator(line.text) {
            if let Some(previous) = out.last_mut() {
                previous.span.end = line.start;
            }
            let after = line.start + 3;
            let raw = &source[after..line.end];
            let trimmed = raw.trim();
            let lead = raw.len() - raw.trim_start().len();
            out.push(Segment {
                lines: Vec::new(),
                name: Some(trimmed.to_string()),
                name_span: Some(after + lead..after + lead + trimmed.len()),
                span: line.start..line.end,
            });
            continue;
        }
        let last = out.last_mut().expect("one segment always exists");
        last.span.end = line.end;
        last.lines.push(line);
    }
    if let Some(last) = out.last_mut() {
        last.span.end = source.len();
    }
    out
}

impl Segment<'_> {
    /// A segment carrying only comments, blanks and `@var` lines declares no
    /// request — it is the file header, or a stray separator.
    pub fn is_declarative(&self) -> bool {
        self.lines.iter().all(|line| {
            line.text.trim().is_empty()
                || is_comment(line.text)
                || var_definition(line.text).is_some()
        })
    }
}

/// Reads a `{% … %}` block starting on `lines[index]` and returns the span of the
/// script text between the delimiters.
pub fn read_script(
    source: &str,
    lines: &[Line<'_>],
    index: usize,
) -> CoreResult<(Range<usize>, usize)> {
    let first = &lines[index];
    let open = source[first.start..first.end]
        .find("{%")
        .ok_or_else(|| parse_err(first.number, "a script block must open with `{%`"))?;
    let body_start = first.start + open + 2;
    if let Some(close) = source[body_start..first.end].find("%}") {
        return Ok((body_start..body_start + close, index + 1));
    }
    let mut cursor = index + 1;
    while cursor < lines.len() {
        let line = &lines[cursor];
        if let Some(close) = source[line.start..line.end].find("%}") {
            return Ok((body_start..line.start + close, cursor + 1));
        }
        cursor += 1;
    }
    Err(parse_err(
        first.number,
        "this script block is never closed with `%}`",
    ))
}

/// The script the engine runs, without the indentation the file gave it.
pub fn dedent(text: &str) -> String {
    let body = text.trim_matches(|c| c == '\r' || c == '\n');
    let indent = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.lines()
        .map(|l| {
            l.char_indices()
                .nth(indent)
                .map(|(at, _)| &l[at..])
                .unwrap_or_else(|| l.trim_start())
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// Replaces only the `{{name}}` templates the map knows about, leaving every other
/// template untouched for the environment to resolve later.
pub fn substitute(text: &str, vars: &BTreeMap<String, String>) -> String {
    if vars.is_empty() || !text.contains("{{") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        match after.find("}}") {
            Some(close) => {
                let name = after[..close].trim();
                match vars.get(name) {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push_str("{{");
                        out.push_str(&after[..close]);
                        out.push_str("}}");
                    }
                }
                rest = &after[close + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Resolves `@var` definitions in declaration order, each against the ones before
/// it, so `@url = {{host}}/v1` works.
pub fn resolve_vars(vars: &[(String, String)]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in vars {
        let resolved = substitute(value, &out);
        out.insert(name.clone(), resolved);
    }
    out
}

/// Applies non-overlapping replacements to `source`, right to left, so every span
/// stays valid while the edit is built.
pub fn splice(source: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = source.to_string();
    for (range, replacement) in edits.into_iter().rev() {
        out.replace_range(range, &replacement);
    }
    out
}

/// Everything a request can carry that the text formats have no line for. Saving
/// one has to fail rather than drop it, or an edit in a GUI would quietly delete
/// what the file never showed.
pub fn reject_inexpressible(request: &SavedRequest, extension: &str) -> CoreResult<()> {
    if !request.tests.is_empty() || !request.captures.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "a .{extension} file cannot carry declarative tests or captures — assert and capture from a `> {{% … %}}` response script instead"
        )));
    }
    Ok(())
}

/// A description has no field of its own, but it does have a home: the comment
/// lines above the request, which is where the format expects prose. Rendering a
/// new block can place it there; editing an existing one cannot, because parsing
/// never reports a comment as a description and a save would duplicate it.
pub fn description_comment(description: Option<&str>, nl: &str) -> String {
    let Some(text) = description.map(str::trim).filter(|d| !d.is_empty()) else {
        return String::new();
    };
    text.lines()
        .map(|line| format!("# {}{nl}", line.trim_end()))
        .collect()
}

pub fn reject_description_edit(request: &SavedRequest, extension: &str) -> CoreResult<()> {
    if request.description.is_some() {
        return Err(CoreError::Unsupported(format!(
            "a .{extension} file keeps a description in the `#` comments above the request, so it cannot be edited as a field — change the comment in the file"
        )));
    }
    Ok(())
}

pub fn indexes_of(names: &[String], fragment: &str, total: usize) -> CoreResult<usize> {
    if let Ok(index) = fragment.parse::<usize>() {
        if index < total {
            return Ok(index);
        }
        return Err(CoreError::NotFound(format!(
            "this file holds {total} requests, so there is no request {index}"
        )));
    }
    let matches: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.as_str() == fragment)
        .map(|(index, _)| index)
        .collect();
    match matches.as_slice() {
        [only] => Ok(*only),
        [] => Err(CoreError::NotFound(format!(
            "no request named {fragment:?} in this file"
        ))),
        _ => Err(CoreError::Conflict(format!(
            "{} requests are named {fragment:?} — address this one by index instead",
            matches.len()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_carry_offsets_for_lf_and_crlf() {
        let lf = lines("a\nbb\n");
        assert_eq!(lf.len(), 2);
        assert_eq!(lf[1].text, "bb");
        assert_eq!(&"a\nbb\n"[lf[1].start..lf[1].end], "bb");

        let crlf = lines("a\r\nbb\r\n");
        assert_eq!(crlf.len(), 2);
        assert_eq!(crlf[1].text, "bb");
        assert_eq!(&"a\r\nbb\r\n"[crlf[1].start..crlf[1].end], "bb");
    }

    #[test]
    fn a_file_without_a_trailing_newline_keeps_its_last_line() {
        let out = lines("a\nb");
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].text, "b");
    }

    #[test]
    fn workspace_relative_rejects_escapes() {
        assert_eq!(
            workspace_relative(3, "./payload.json", "the body file").unwrap(),
            "payload.json"
        );
        for bad in ["/etc/passwd", "../secret", "a/../../b", "C:/x"] {
            assert_eq!(
                workspace_relative(3, bad, "the body file")
                    .unwrap_err()
                    .code(),
                "E_PATH_ESCAPE",
                "{bad}"
            );
        }
    }

    #[test]
    fn substitute_leaves_unknown_templates_alone() {
        let vars = BTreeMap::from([("host".to_string(), "x.dev".to_string())]);
        assert_eq!(substitute("{{host}}/{{path}}", &vars), "x.dev/{{path}}");
    }

    #[test]
    fn vars_resolve_against_earlier_definitions() {
        let out = resolve_vars(&[
            ("host".to_string(), "x.dev".to_string()),
            ("base".to_string(), "https://{{host}}/v1".to_string()),
        ]);
        assert_eq!(out["base"], "https://x.dev/v1");
    }

    #[test]
    fn splice_applies_every_edit() {
        let out = splice(
            "abcdef",
            vec![(0..1, "A".to_string()), (4..6, "EF".to_string())],
        );
        assert_eq!(out, "AbcdEF");
    }

    #[test]
    fn dedent_strips_the_common_indentation() {
        assert_eq!(dedent("\n  a\n    b\n  "), "a\n  b");
    }
}
