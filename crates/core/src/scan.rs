use crate::error::{CoreError, CoreResult};
use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub path: PathBuf,
    pub line: usize,
    pub rule: &'static str,
    pub excerpt: String,
}

struct Rule {
    name: &'static str,
    pattern: Regex,
    /// A second look at what the pattern found, for the shapes a regex cannot
    /// describe on its own.
    refine: Option<fn(&str) -> bool>,
}

/// Base32 payloads carry digits. `GETHEADPOSTPUTDELETEPATCHOPTIONS` does not,
/// and neither does a run of shouting constants.
fn is_base32_payload(token: &str) -> bool {
    token.chars().filter(|c| matches!(c, '2'..='7')).count() >= 2
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (
                "jwt",
                r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
            ),
            ("stripe-live-key", r"sk_live_[0-9A-Za-z]{10,}"),
            ("github-token", r"gh[pousr]_[0-9A-Za-z]{20,}"),
            ("aws-access-key", r"(AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}"),
            ("google-api-key", r"AIza[0-9A-Za-z_\-]{35}"),
            ("private-key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
            ("gitlab-token", r"glpat-[0-9A-Za-z_\-]{20,}"),
            ("slack-token", r"xox[baprs]-[0-9A-Za-z\-]{10,}"),
            ("npm-token", r"npm_[0-9A-Za-z]{36}"),
            ("anthropic-key", r"sk-ant-[0-9A-Za-z_\-]{20,}"),
            ("openai-key", r"sk-proj-[0-9A-Za-z_\-]{20,}"),
            (
                "credential-in-url",
                r"[a-zA-Z][a-zA-Z0-9+.\-]*://[^/\s:@]+:[^/\s@]{6,}@",
            ),
            ("base32-token", r"[A-Z2-7]{32,}={0,6}"),
        ]
        .into_iter()
        .map(|(name, pattern)| Rule {
            name,
            pattern: Regex::new(pattern).expect("static scan pattern"),
            refine: match name {
                "base32-token" => Some(is_base32_payload),
                _ => None,
            },
        })
        .collect()
    })
}

/// `.` and `:` belong inside the run: without them a JWT, a `user:pass` pair and
/// every dotted token split into short halves that no later check could see.
fn candidate() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/=_.:\-]{20,}").expect("static candidate pattern"))
}

/// `name = value` and `name: value`, the shape every configuration format a
/// workspace can hold agrees on.
fn assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"([A-Za-z0-9_.\-]{2,64})"?\s*[:=]\s*"?([A-Za-z0-9+/=_@%~\-][A-Za-z0-9+/=_.:@%~\-]{7,})"#,
        )
        .expect("static assignment pattern")
    })
}

/// Every place a value can sit: the contents of a quoted literal, and the right
/// hand side of an assignment. The entropy checks look **here** and nowhere
/// else — run loose over prose or over source code they flag `serde_json::Value`
/// and every other long identifier, and a scanner nobody believes gets removed.
fn values_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find(['"', '\'']) {
        let quote = rest.as_bytes()[open] as char;
        let after = &rest[open + 1..];
        let Some(close) = after.find(quote) else {
            break;
        };
        out.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    for capture in assignment().captures_iter(line) {
        let (Some(key), Some(value)) = (capture.get(1), capture.get(2)) else {
            continue;
        };
        if line[key.end()..value.start()].contains("::") {
            continue;
        }
        out.push(value.as_str().to_string());
    }
    out
}

fn secretish_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(pass(word|wd|phrase)?|secret|token|api[_.\-]?key|apikey|access[_.\-]?key|auth|credential|bearer|private[_.\-]?key|signing[_.\-]?key)",
        )
        .expect("static key pattern")
    })
}

fn shannon_bits_per_char(text: &str) -> f64 {
    let total = text.chars().count() as f64;
    if total == 0.0 {
        return 0.0;
    }
    let mut counts = std::collections::BTreeMap::new();
    for c in text.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    -counts
        .values()
        .map(|n| {
            let p = *n as f64 / total;
            p * p.log2()
        })
        .sum::<f64>()
}

fn character_classes(token: &str) -> usize {
    [
        token.chars().any(|c| c.is_ascii_lowercase()),
        token.chars().any(|c| c.is_ascii_uppercase()),
        token.chars().any(|c| c.is_ascii_digit()),
        token.chars().any(|c| matches!(c, '+' | '/' | '_' | '-')),
    ]
    .into_iter()
    .filter(|present| *present)
    .count()
}

fn is_uuid(token: &str) -> bool {
    let groups: Vec<usize> = token.split('-').map(str::len).collect();
    groups == [8, 4, 4, 4, 12] && token.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// A value the author wrote to be replaced, or a shape a workspace is simply
/// full of. Naming them is what keeps the scanner quiet enough to stay on.
fn is_benign(token: &str) -> bool {
    if token.contains("{{") || token.contains("://") || token.starts_with('$') {
        return true;
    }
    if is_uuid(token) {
        return true;
    }
    let lower = token.to_ascii_lowercase();
    if [
        "example",
        "changeme",
        "placeholder",
        "your-",
        "your_",
        "redacted",
        "dummy",
        "sample",
        "xxxx",
        "....",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return true;
    }
    let punctuation_only = token
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | '.' | '+'));
    let path_shaped = token.contains('/') && !token.contains('+') && !token.contains('=');
    punctuation_only || path_shaped || is_identifier_shaped(token)
}

/// `EnvVarStore::variable_name`, `users-list-fixture`, `mock.v1.Mock/Unary`:
/// several separated runs that each read as a word. Nobody's credential is
/// spelled in words, and everybody's source, path and prose is.
fn is_identifier_shaped(token: &str) -> bool {
    let segments: Vec<&str> = token
        .split(['_', '-', '.', ':', '/', '+', '@'])
        .filter(|s| !s.is_empty())
        .collect();
    segments.len() >= 2 && segments.iter().all(|s| is_word_shaped(s))
}

/// A word, optionally versioned: `orders`, `v2`, `sha256`. Digits mixed *into*
/// the letters is what a credential looks like and a word never does.
fn is_word_shaped(segment: &str) -> bool {
    segment.starts_with(|c: char| c.is_ascii_alphabetic())
        && segment.chars().all(|c| c.is_ascii_alphanumeric())
        && segment
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .chars()
            .all(|c| c.is_ascii_alphabetic())
}

fn looks_high_entropy(token: &str) -> bool {
    if token.chars().count() < 20 || is_benign(token) {
        return false;
    }
    let classes = character_classes(token);
    let bits = shannon_bits_per_char(token);
    (classes >= 3 && bits >= 3.2) || (classes >= 2 && bits >= 4.0)
}

/// Letters that read as Latin but are not. Folding them back is what makes a
/// key spelled with a Cyrillic `А` visible to an ASCII rule again.
fn deconfuse(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'А' | 'Α' => 'A',
            'В' | 'Β' => 'B',
            'С' | 'Ϲ' => 'C',
            'Е' | 'Ε' => 'E',
            'Н' | 'Η' => 'H',
            'І' | 'Ι' => 'I',
            'Ј' => 'J',
            'К' | 'Κ' => 'K',
            'М' | 'Μ' => 'M',
            'Ν' => 'N',
            'О' | 'Ο' => 'O',
            'Р' | 'Ρ' => 'P',
            'Ѕ' => 'S',
            'Т' | 'Τ' => 'T',
            'Х' | 'Χ' => 'X',
            'У' | 'Υ' => 'Y',
            'Ζ' => 'Z',
            'а' => 'a',
            'с' => 'c',
            'е' => 'e',
            'і' => 'i',
            'ј' => 'j',
            'о' => 'o',
            'р' => 'p',
            'ѕ' => 's',
            'х' => 'x',
            'у' => 'y',
            other => other,
        })
        .collect()
}

/// A run long enough to be a credential, whatever alphabet it is spelled in.
/// The candidate pattern is ASCII by design, so a token carrying a look-alike
/// letter needs its own reader.
fn unbroken_run() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"[^\s"',;=(){}\[\]]{20,}"#).expect("static run pattern"))
}

fn is_mixed_script(token: &str) -> bool {
    let ascii = token.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    let foreign = token
        .chars()
        .any(|c| c.is_alphabetic() && !c.is_ascii_alphabetic());
    foreign && ascii >= 12 && !is_benign(token)
}

/// The contents of every quoted run on the line, joined. A token an author split
/// across two literals to slip past a reader that works a line at a time arrives
/// whole here.
fn quoted_runs(line: &str) -> Option<String> {
    let mut joined = String::new();
    let mut runs = 0usize;
    let mut rest = line;
    while let Some(open) = rest.find(['"', '\'']) {
        let quote = rest.as_bytes()[open] as char;
        let after = &rest[open + 1..];
        let Some(close) = after.find(quote) else {
            break;
        };
        joined.push_str(&after[..close]);
        runs += 1;
        rest = &after[close + 1..];
    }
    (runs >= 2).then_some(joined)
}

/// A credential someone base64'd is still a credential, so every candidate that
/// decodes to printable text is scanned as text too.
fn decoded_candidates(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for found in candidate().find_iter(line).take(16) {
        let raw = found.as_str().trim_end_matches('=');
        if raw.len() < 20 || raw.contains(['.', ':', '-']) {
            continue;
        }
        let Ok(bytes) = STANDARD_NO_PAD.decode(raw) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if text.len() >= 12 && text.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            out.push(text);
        }
    }
    out
}

fn excerpt(raw: &str) -> String {
    let head: String = raw.chars().take(6).collect();
    format!("{head}… ({} chars)", raw.chars().count())
}

const MAX_JOINED_LINES: usize = 8;
const MAX_LOGICAL_LEN: usize = 8 * 1024;

/// The lines a reader sees, joined back into the values a machine sees: a `\`,
/// `+` or `,` continuation, and two adjacent literals, are one token in two.
fn logical_lines(text: &str) -> Vec<(usize, String)> {
    let physical: Vec<&str> = text.lines().collect();
    let mut out = Vec::with_capacity(physical.len());
    for (index, first) in physical.iter().enumerate() {
        let mut joined = first.to_string();
        let mut span = 1usize;
        while span < MAX_JOINED_LINES && index + span < physical.len() {
            let current = joined.trim_end();
            let next = physical[index + span].trim_start();
            let continues = current.ends_with(['\\', '+', ','])
                || (current.ends_with(['"', '\'']) && next.starts_with(['"', '\'']));
            if !continues || joined.len() > MAX_LOGICAL_LEN {
                break;
            }
            joined = format!("{}{next}", current.trim_end_matches('\\'));
            span += 1;
        }
        out.push((index + 1, joined));
    }
    out
}

/// A value under a name that says what it is. This is what catches the shapes no
/// pattern can describe — a hex digest, a base32 blob, a vendor's own format.
fn keyed_credential(line: &str) -> Option<String> {
    for capture in assignment().captures_iter(line) {
        let (Some(name), Some(found)) = (capture.get(1), capture.get(2)) else {
            continue;
        };
        if line[name.end()..found.start()].contains("::") {
            continue;
        }
        let key = name.as_str();
        let value = found.as_str().trim_matches(['"', '\'']);
        if !secretish_key().is_match(key) || value.chars().count() < 16 || is_benign(value) {
            continue;
        }
        let prose = value
            .chars()
            .all(|c| c.is_ascii_lowercase() || matches!(c, '-' | '_'));
        if prose {
            continue;
        }
        return Some(value.to_string());
    }
    None
}

/// Binary bytes read as text produce endless look-alike runs and endless
/// high-entropy runs, none of which mean anything. A credential *pattern* still
/// means something there, so the named rules run and the guesswork does not.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Depth {
    Patterns,
    Guesses,
}

pub fn scan_text(path: &Path, text: &str) -> Vec<Finding> {
    scan_lines(path, text, Depth::Guesses)
}

fn scan_lines(path: &Path, text: &str, depth: Depth) -> Vec<Finding> {
    let mut out = scan_name(path);
    for (number, line) in logical_lines(text) {
        let mut hits: BTreeSet<&'static str> = BTreeSet::new();
        let mut variants = vec![line.clone()];
        let folded = deconfuse(&line);
        if folded != line {
            variants.push(folded);
        }
        if let Some(joined) = quoted_runs(&line) {
            variants.push(joined);
        }
        variants.extend(decoded_candidates(&line));

        for variant in &variants {
            for rule in rules() {
                let hit = rule
                    .pattern
                    .find_iter(variant)
                    .find(|f| rule.refine.is_none_or(|refine| refine(f.as_str())));
                if let Some(found) = hit {
                    if hits.insert(rule.name) {
                        out.push(Finding {
                            path: path.to_path_buf(),
                            line: number,
                            rule: rule.name,
                            excerpt: excerpt(found.as_str()),
                        });
                    }
                }
            }
        }
        if !hits.is_empty() || depth == Depth::Patterns {
            continue;
        }

        if let Some(found) = unbroken_run()
            .find_iter(&line)
            .find(|f| is_mixed_script(f.as_str()))
        {
            out.push(Finding {
                path: path.to_path_buf(),
                line: number,
                rule: "mixed-script-token",
                excerpt: excerpt(found.as_str()),
            });
            continue;
        }

        if let Some(found) = values_in(&line)
            .iter()
            .flat_map(|value| {
                candidate()
                    .find_iter(value)
                    .map(|f| f.as_str().to_string())
                    .collect::<Vec<_>>()
            })
            .find(|token| looks_high_entropy(token))
        {
            out.push(Finding {
                path: path.to_path_buf(),
                line: number,
                rule: "high-entropy",
                excerpt: excerpt(&found),
            });
            continue;
        }

        if let Some(value) = keyed_credential(&line) {
            out.push(Finding {
                path: path.to_path_buf(),
                line: number,
                rule: "credential-assignment",
                excerpt: excerpt(&value),
            });
        }
    }
    out
}

/// A name can be the whole finding: `id_rsa` and `prod.pem` carry a credential
/// whatever their bytes turn out to say.
fn scan_name(path: &Path) -> Vec<Finding> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let lower = name.to_ascii_lowercase();
    let known = [".pem", ".p12", ".pfx", ".jks", ".keystore", ".ppk"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        || ["id_rsa", "id_ed25519", "id_ecdsa", "credentials"].contains(&lower.as_str());
    let mut out = Vec::new();
    if known {
        out.push(Finding {
            path: path.to_path_buf(),
            line: 1,
            rule: "credential-file-name",
            excerpt: format!("{name} is a key or credential file"),
        });
    }
    for rule in rules() {
        let hit = rule
            .pattern
            .find_iter(name)
            .find(|f| rule.refine.is_none_or(|refine| refine(f.as_str())));
        if let Some(found) = hit {
            out.push(Finding {
                path: path.to_path_buf(),
                line: 1,
                rule: rule.name,
                excerpt: excerpt(found.as_str()),
            });
        }
    }
    out
}

/// The values this machine keeps to itself live outside every workspace. One of
/// these files turning up *inside* a repository is a mistake, and committing it
/// is the catastrophic case — so the scanner names the file instead of reading
/// it, and the staging path refuses it outright.
pub fn is_local_values_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|raw| {
            let name = raw.to_ascii_lowercase();
            name == "secrets.toml"
                || name == ".secrets.toml"
                || name == ".env"
                || name.starts_with(".env.")
                || name.ends_with(".local.toml")
                || name.ends_with(".secret.toml")
        })
        .unwrap_or(false)
}

/// `.git` is the one thing a scan skips, and it is skipped because nothing
/// inside it is ever committed. Every other dotfile — `.secrets.toml`, `.env`,
/// `.github/workflows` — commits like any other file, so it is read like one.
fn is_git_internals(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new(".git"))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> CoreResult<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| CoreError::io(dir.display(), e))? {
        let path = entry.map_err(|e| CoreError::io(dir.display(), e))?.path();
        if path.file_name() == Some(std::ffi::OsStr::new(".git")) || path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

pub fn scan_workspace(ws: &Path) -> CoreResult<Vec<Finding>> {
    let mut files = Vec::new();
    walk(ws, &mut files)?;
    files.sort();
    scan_files(&files)
}

/// Bytes that are not UTF-8 are read anyway, lossily. A control that gives up on
/// what it cannot read is a control an attacker turns off with one stray byte.
pub fn scan_bytes(path: &Path, bytes: &[u8]) -> Vec<Finding> {
    let depth = if looks_binary(bytes) {
        Depth::Patterns
    } else {
        Depth::Guesses
    };
    scan_lines(path, &String::from_utf8_lossy(bytes), depth)
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|byte| *byte == 0)
}

fn local_values_finding(path: &Path) -> Finding {
    Finding {
        path: path.to_path_buf(),
        line: 1,
        rule: "local-values-file",
        excerpt: "holds values this machine keeps to itself and must never be committed"
            .to_string(),
    }
}

fn unreadable_finding(path: &Path, why: impl std::fmt::Display) -> Finding {
    Finding {
        path: path.to_path_buf(),
        line: 1,
        rule: "unreadable",
        excerpt: format!("cannot be read, so it cannot be cleared: {why}"),
    }
}

pub fn scan_files(files: &[PathBuf]) -> CoreResult<Vec<Finding>> {
    let mut findings = Vec::new();
    for file in files {
        if !file.is_file() || is_git_internals(file) {
            continue;
        }
        if is_local_values_file(file) {
            findings.push(local_values_finding(file));
            continue;
        }
        match std::fs::read(file) {
            Ok(bytes) => findings.extend(scan_bytes(file, &bytes)),
            Err(e) => findings.push(unreadable_finding(file, e)),
        }
    }
    Ok(findings)
}

fn repo_of(root: &Path) -> CoreResult<git2::Repository> {
    // git prints its whole usage page to stderr when it is not in a repository.
    // Saying it in one line here is the difference between a hint and 180 lines.
    git2::Repository::discover(root).map_err(|_| {
        CoreError::NotFound(format!(
            "{} is not inside a git repository, so there is nothing staged — run `git init` there, or scan the whole workspace with `mandalo scan`",
            root.display()
        ))
    })
}

fn staged_paths(repo: &git2::Repository) -> CoreResult<Vec<PathBuf>> {
    let index = repo
        .index()
        .map_err(|e| CoreError::Io(format!("cannot read the git index: {e}")))?;
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let diff = repo
        .diff_tree_to_index(head.as_ref(), Some(&index), None)
        .map_err(|e| CoreError::Io(format!("cannot compare the git index: {e}")))?;
    let mut out = BTreeSet::new();
    for delta in diff.deltas() {
        if delta.status() == git2::Delta::Deleted {
            continue;
        }
        if let Some(path) = delta.new_file().path() {
            out.insert(path.to_path_buf());
        }
    }
    Ok(out.into_iter().collect())
}

pub fn staged_files(root: &Path) -> CoreResult<Vec<PathBuf>> {
    let repo = repo_of(root)?;
    Ok(staged_paths(&repo)?
        .into_iter()
        .map(|p| root.join(p))
        .collect())
}

/// Reads what is **staged**, never what is on disk. Staging a token and then
/// overwriting the working copy left the token in the index and in the commit,
/// with the hook reporting nothing at all.
pub fn scan_staged(root: &Path) -> CoreResult<Vec<Finding>> {
    let repo = repo_of(root)?;
    let index = repo
        .index()
        .map_err(|e| CoreError::Io(format!("cannot read the git index: {e}")))?;
    let mut findings = Vec::new();
    for path in staged_paths(&repo)? {
        if is_git_internals(&path) {
            continue;
        }
        if is_local_values_file(&path) {
            findings.push(local_values_finding(&path));
            continue;
        }
        let Some(entry) = index.get_path(&path, 0) else {
            continue;
        };
        if entry.mode == 0o160000 {
            continue;
        }
        match repo.find_blob(entry.id) {
            Ok(blob) => findings.extend(scan_bytes(&path, blob.content())),
            Err(e) => findings.push(unreadable_finding(&path, e)),
        }
    }
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_hit(text: &str) -> Vec<&'static str> {
        scan_text(Path::new("x.toml"), text)
            .into_iter()
            .map(|f| f.rule)
            .collect()
    }

    /// Credential-shaped fixtures are assembled at runtime: as source literals they
    /// trip GitHub's own push protection, which cannot tell a documentation example
    /// from a live key.
    fn aws_fixture() -> String {
        format!("AKIA{}", "IOSFODNN7EXAMPLE")
    }

    #[test]
    fn detects_the_documented_credential_shapes() {
        assert_eq!(
            rules_hit("token = \"eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U\""),
            vec!["jwt"]
        );
        assert_eq!(
            rules_hit(&format!("key = \"sk_{}_4eC39HqLyjWDarjtT1zdp7dc\"", "live")),
            vec!["stripe-live-key"]
        );
        assert_eq!(
            rules_hit(&format!(
                "gh = \"gh{}_16C7e42F292c6912E7710c838347Ae178B4a\"",
                "p"
            )),
            vec!["github-token"]
        );
        assert_eq!(
            rules_hit(&format!("aws = \"{}\"", aws_fixture())),
            vec!["aws-access-key"]
        );
        assert_eq!(
            rules_hit(&format!(
                "g = \"AIza{}-9tSrke72PouQMnMX-a7eZSW0jkFMBWY\"",
                "SyD"
            )),
            vec!["google-api-key"]
        );
        assert_eq!(
            rules_hit("-----BEGIN RSA PRIVATE KEY-----"),
            vec!["private-key"]
        );
    }

    #[test]
    fn flags_high_entropy_literals() {
        let hits = rules_hit("secret = \"Xq7Lm2Pv9Zt4Rn8Wb3Kd6Yh1\"");
        assert_eq!(hits, vec!["high-entropy"]);
    }

    #[test]
    fn leaves_ordinary_workspace_files_alone() {
        let clean = r#"
id = "550e8400-e29b-41d4-a716-446655440000"
name = "List orders"
url = "{{base}}/orders?limit=25"
description = "returns the most recent orders for the signed in user"
"#;
        let found = scan_text(Path::new("req.toml"), clean);
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn reports_path_line_and_a_truncated_excerpt() {
        let found = scan_text(
            Path::new("envs/prod.toml"),
            &format!("name = \"prod\"\ntoken = \"{}\"\n", aws_fixture()),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert_eq!(found[0].path, Path::new("envs/prod.toml"));
        assert_eq!(found[0].excerpt, "AKIAIO… (20 chars)");
    }

    #[test]
    fn scans_a_whole_workspace_and_skips_only_git_internals() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), aws_fixture()).unwrap();
        std::fs::write(dir.path().join("clean.toml"), "name = \"prod\"").unwrap();
        std::fs::write(
            dir.path().join("dirty.toml"),
            format!("k = \"{}\"", aws_fixture()),
        )
        .unwrap();
        let found = scan_workspace(dir.path()).unwrap();
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].path.ends_with("dirty.toml"));
    }

    #[test]
    fn a_dotfile_is_scanned_like_any_other_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".aws-config"),
            format!("key = \"{}\"", aws_fixture()),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        std::fs::write(
            dir.path().join(".github/workflows/ci.yml"),
            format!("  KEY: {}", aws_fixture()),
        )
        .unwrap();
        let found = scan_workspace(dir.path()).unwrap();
        assert_eq!(found.len(), 2, "{found:#?}");
    }

    #[test]
    fn a_dot_secrets_file_is_named_instead_of_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".secrets.toml"), aws_fixture()).unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=whatever").unwrap();
        let found = scan_workspace(dir.path()).unwrap();
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found.iter().all(|f| f.rule == "local-values-file"));
        assert!(is_local_values_file(Path::new(".secrets.toml")));
        assert!(is_local_values_file(Path::new(".env.production")));
    }

    #[test]
    fn one_stray_byte_cannot_hide_a_file_from_the_scanner() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = format!("k = \"{}\"\n", aws_fixture()).into_bytes();
        bytes.push(0xff);
        std::fs::write(dir.path().join("binary.toml"), bytes).unwrap();
        let found = scan_workspace(dir.path()).unwrap();
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].rule, "aws-access-key");
    }

    #[test]
    fn a_token_split_across_two_lines_is_still_found() {
        for source in [
            format!("token = \"AKIA\" \\\n  \"{}\"", "IOSFODNN7EXAMPLE"),
            format!("token = \"AKIA\" +\n  \"{}\"", "IOSFODNN7EXAMPLE"),
            format!("token = (\"AKIA\"\n  \"{}\")", "IOSFODNN7EXAMPLE"),
        ] {
            assert!(
                rules_hit(&source).contains(&"aws-access-key"),
                "{source:?} -> {:?}",
                rules_hit(&source)
            );
        }
    }

    #[test]
    fn a_look_alike_letter_does_not_hide_a_key() {
        let cyrillic = format!("aws = \"AKI\u{0410}{}\"", "IOSFODNN7EXAMPLE");
        let hits = rules_hit(&cyrillic);
        assert!(
            hits.contains(&"aws-access-key") || hits.contains(&"mixed-script-token"),
            "{hits:?}"
        );
    }

    #[test]
    fn a_base64_wrapped_key_is_decoded_and_found() {
        let wrapped = base64::engine::general_purpose::STANDARD.encode(aws_fixture().as_bytes());
        assert!(
            rules_hit(&format!("blob = \"{wrapped}\"")).contains(&"aws-access-key"),
            "{wrapped}"
        );
    }

    #[test]
    fn a_token_carrying_a_dot_or_a_colon_is_not_split_out_of_sight() {
        let dotted = rules_hit("api_key = \"7f3a2b1c.9d8e7f6a5b4c3d2e1f0a9b8c\"");
        assert!(!dotted.is_empty(), "{dotted:?}");
        assert!(
            rules_hit("url = \"https://admin:hunter2hunter2@internal.acme.io/\"")
                .contains(&"credential-in-url")
        );
    }

    #[test]
    fn a_lowercase_hex_token_under_a_credential_key_is_reported() {
        let hits = rules_hit("gitlab_token = 4f2a1b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a");
        assert!(!hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn an_uppercase_base32_secret_is_reported() {
        let hits = rules_hit("totp = \"MFRGGZDFMZTWQ2LKNNWG23TPOBYXE43UOJUW4ZY\"");
        assert!(hits.contains(&"base32-token"), "{hits:?}");
    }

    #[test]
    fn a_key_shaped_file_name_is_a_finding_on_its_own() {
        let found = scan_text(Path::new("keys/id_rsa"), "");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule, "credential-file-name");
        assert_eq!(scan_text(Path::new("keys/server.pem"), "").len(), 1);
    }

    #[test]
    fn staged_files_outside_a_repository_says_so_in_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let error = staged_files(dir.path()).unwrap_err();
        assert_eq!(error.code(), "E_NOT_FOUND");
        let message = error.to_string();
        assert!(message.contains("not inside a git repository"), "{message}");
        assert_eq!(message.lines().count(), 1, "{message}");
    }

    #[test]
    fn the_staged_scan_reads_the_index_not_the_working_copy() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let file = dir.path().join("env.toml");
        std::fs::write(&file, format!("k = \"{}\"\n", aws_fixture())).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("env.toml")).unwrap();
        index.write().unwrap();
        std::fs::write(&file, "k = \"nothing to see here\"\n").unwrap();

        assert!(
            scan_files(std::slice::from_ref(&file)).unwrap().is_empty(),
            "the working copy is clean, which is the whole trick"
        );
        let staged = scan_staged(dir.path()).unwrap();
        assert_eq!(staged.len(), 1, "{staged:#?}");
        assert_eq!(staged[0].rule, "aws-access-key");
        assert_eq!(staged[0].path, PathBuf::from("env.toml"));
    }

    #[test]
    fn the_scanner_stays_quiet_on_the_example_workspace() {
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/mock-workspace")
            .canonicalize();
        let Ok(example) = example else {
            return;
        };
        let found = scan_workspace(&example).unwrap();
        assert!(found.is_empty(), "{found:#?}");
    }
}
