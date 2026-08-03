use crate::error::{CoreError, CoreResult};
use regex::Regex;
use serde::Serialize;
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
            ("aws-access-key", r"AKIA[0-9A-Z]{16}"),
            ("google-api-key", r"AIza[0-9A-Za-z_\-]{35}"),
            ("private-key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ]
        .into_iter()
        .map(|(name, pattern)| Rule {
            name,
            pattern: Regex::new(pattern).expect("static scan pattern"),
        })
        .collect()
    })
}

fn candidate() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z0-9+/=_\-]{20,}").expect("static candidate pattern"))
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

fn looks_high_entropy(token: &str) -> bool {
    let has_lower = token.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = token.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    has_lower && has_upper && has_digit && shannon_bits_per_char(token) >= 4.0
}

pub fn scan_text(path: &Path, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let mut matched = false;
        for rule in rules() {
            if let Some(found) = rule.pattern.find(line) {
                out.push(Finding {
                    path: path.to_path_buf(),
                    line: index + 1,
                    rule: rule.name,
                    excerpt: excerpt(found.as_str()),
                });
                matched = true;
            }
        }
        if matched {
            continue;
        }
        for found in candidate().find_iter(line) {
            if looks_high_entropy(found.as_str()) {
                out.push(Finding {
                    path: path.to_path_buf(),
                    line: index + 1,
                    rule: "high-entropy",
                    excerpt: excerpt(found.as_str()),
                });
                break;
            }
        }
    }
    out
}

fn excerpt(raw: &str) -> String {
    let head: String = raw.chars().take(6).collect();
    format!("{head}… ({} chars)", raw.chars().count())
}

fn is_scannable(path: &Path) -> bool {
    !path
        .file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(true)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> CoreResult<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| CoreError::io(dir.display(), e))? {
        let path = entry.map_err(|e| CoreError::io(dir.display(), e))?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') {
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

pub fn scan_files(files: &[PathBuf]) -> CoreResult<Vec<Finding>> {
    let mut findings = Vec::new();
    for file in files {
        if !file.is_file() || !is_scannable(file) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        findings.extend(scan_text(file, &text));
    }
    Ok(findings)
}

pub fn staged_files(root: &Path) -> CoreResult<Vec<PathBuf>> {
    let output = std::process::Command::new("git")
        .args(["diff", "--cached", "--name-only", "--diff-filter=ACMR"])
        .current_dir(root)
        .output()
        .map_err(|e| CoreError::Io(format!("cannot run git: {e}")))?;
    if !output.status.success() {
        return Err(CoreError::Io(format!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| root.join(l))
        .collect())
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
            rules_hit(&format!("gh = \"gh{}_16C7e42F292c6912E7710c838347Ae178B4a\"", "p")),
            vec!["github-token"]
        );
        assert_eq!(
            rules_hit(&format!("aws = \"{}\"", aws_fixture())),
            vec!["aws-access-key"]
        );
        assert_eq!(
            rules_hit(&format!("g = \"AIza{}-9tSrke72PouQMnMX-a7eZSW0jkFMBWY\"", "SyD")),
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
        assert!(scan_text(Path::new("req.toml"), clean).is_empty());
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
    fn scans_a_whole_workspace_and_skips_dot_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), &aws_fixture()).unwrap();
        std::fs::write(dir.path().join("clean.toml"), "name = \"prod\"").unwrap();
        std::fs::write(
            dir.path().join("dirty.toml"),
            &format!("k = \"{}\"", aws_fixture()),
        )
        .unwrap();
        let found = scan_workspace(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("dirty.toml"));
    }
}
