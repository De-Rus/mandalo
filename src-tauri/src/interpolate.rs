use std::collections::BTreeMap;

pub fn apply(template: &str, vars: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| format!("unclosed {{{{ in: {template}"))?;
        let key = after[..end].trim();
        let value = vars
            .get(key)
            .ok_or_else(|| format!("unresolved variable: {key}"))?;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn passthrough_without_variables() {
        assert_eq!(apply("https://api.example.com", &vars(&[])).unwrap(), "https://api.example.com");
    }

    #[test]
    fn substitutes_single_variable() {
        let v = vars(&[("base", "https://api.example.com")]);
        assert_eq!(apply("{{base}}/users", &v).unwrap(), "https://api.example.com/users");
    }

    #[test]
    fn substitutes_multiple_and_repeated() {
        let v = vars(&[("host", "x.dev"), ("v", "v2")]);
        assert_eq!(
            apply("https://{{host}}/{{v}}/{{v}}", &v).unwrap(),
            "https://x.dev/v2/v2"
        );
    }

    #[test]
    fn trims_whitespace_inside_braces() {
        let v = vars(&[("token", "abc")]);
        assert_eq!(apply("Bearer {{ token }}", &v).unwrap(), "Bearer abc");
    }

    #[test]
    fn unresolved_variable_fails_loud() {
        let err = apply("{{base}}/users", &vars(&[])).unwrap_err();
        assert!(err.contains("unresolved variable: base"));
    }

    #[test]
    fn unclosed_braces_fail_loud() {
        let err = apply("{{base/users", &vars(&[("base", "x")])).unwrap_err();
        assert!(err.contains("unclosed"));
    }
}
