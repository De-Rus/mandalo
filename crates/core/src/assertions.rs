use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json_path::JsonPath;
use std::fmt;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scripts {
    #[serde(default)]
    pub pre: Option<String>,
    #[serde(default)]
    pub post: Option<String>,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.pre.is_none() && self.post.is_none()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum StatusOp {
    Eq,
    Ne,
    Lt,
    Gt,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JsonOp {
    Eq,
    Ne,
    Exists,
    Absent,
    Contains,
    Matches,
    Lt,
    Gt,
    Len,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum HeaderOp {
    Exists,
    Absent,
    Eq,
    Contains,
    Matches,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum DurationOp {
    Lt,
    Gt,
}

impl fmt::Display for StatusOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StatusOp::Eq => "eq",
            StatusOp::Ne => "ne",
            StatusOp::Lt => "lt",
            StatusOp::Gt => "gt",
        })
    }
}

impl fmt::Display for JsonOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            JsonOp::Eq => "eq",
            JsonOp::Ne => "ne",
            JsonOp::Exists => "exists",
            JsonOp::Absent => "absent",
            JsonOp::Contains => "contains",
            JsonOp::Matches => "matches",
            JsonOp::Lt => "lt",
            JsonOp::Gt => "gt",
            JsonOp::Len => "len",
        })
    }
}

impl fmt::Display for HeaderOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            HeaderOp::Exists => "exists",
            HeaderOp::Absent => "absent",
            HeaderOp::Eq => "eq",
            HeaderOp::Contains => "contains",
            HeaderOp::Matches => "matches",
        })
    }
}

impl fmt::Display for DurationOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DurationOp::Lt => "lt",
            DurationOp::Gt => "gt",
        })
    }
}

mod millis {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u128, s: S) -> Result<S::Ok, S::Error> {
        let narrowed = u64::try_from(*value).map_err(serde::ser::Error::custom)?;
        s.serialize_u64(narrowed)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        Ok(u128::from(u64::deserialize(d)?))
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TestAssertion {
    Status {
        op: StatusOp,
        value: u16,
    },
    Json {
        path: String,
        op: JsonOp,
        #[serde(default)]
        value: Option<Value>,
    },
    Header {
        name: String,
        op: HeaderOp,
        #[serde(default)]
        value: Option<String>,
    },
    Duration {
        op: DurationOp,
        #[serde(with = "millis")]
        value: u128,
    },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum CaptureScope {
    Run,
    Session,
    Persist,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub from: String,
    pub into: String,
    pub scope: CaptureScope,
}

pub enum CaptureSource {
    Status,
    Header(String),
    Body(JsonPath),
}

pub fn parse_capture_source(from: &str) -> CoreResult<CaptureSource> {
    if from == "status" {
        return Ok(CaptureSource::Status);
    }
    if let Some(name) = from.strip_prefix("header.") {
        if name.is_empty() {
            return Err(CoreError::Capture(format!(
                "capture source has an empty header name: {from:?}"
            )));
        }
        return Ok(CaptureSource::Header(name.to_string()));
    }
    if let Some(path) = from.strip_prefix("body.") {
        if !path.starts_with('$') {
            return Err(CoreError::Capture(format!(
                "capture source body path must start with $: {from:?}"
            )));
        }
        let parsed = JsonPath::parse(path)
            .map_err(|e| CoreError::Capture(format!("invalid JSONPath in {from:?}: {e}")))?;
        return Ok(CaptureSource::Body(parsed));
    }
    Err(CoreError::Capture(format!(
        "invalid capture source: {from:?} (expected status, header.<Name> or body.$.<jsonpath>)"
    )))
}

pub fn validate_capture(capture: &Capture) -> CoreResult<()> {
    parse_capture_source(&capture.from)?;
    if capture.into.is_empty()
        || !capture
            .into
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CoreError::InvalidName(format!(
            "invalid capture target: {:?}",
            capture.into
        )));
    }
    Ok(())
}

pub fn resolve_capture(
    from: &str,
    status: u16,
    headers: &[(String, String)],
    body: &str,
) -> CoreResult<Option<String>> {
    match parse_capture_source(from)? {
        CaptureSource::Status => Ok(Some(status.to_string())),
        CaptureSource::Header(name) => Ok(headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&name))
            .map(|(_, v)| v.clone())),
        CaptureSource::Body(path) => {
            let json: Value = serde_json::from_str(body).map_err(|e| {
                CoreError::Capture(format!(
                    "response body is not JSON, cannot capture {from:?}: {e}"
                ))
            })?;
            Ok(path.query(&json).first().map(node_to_string))
        }
    }
}

fn node_to_string(node: &Value) -> String {
    match node {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64()
}

fn json_eq(node: &Value, expected: &Value) -> bool {
    match (number(node), number(expected)) {
        (Some(a), Some(b)) => a == b,
        _ => node == expected,
    }
}

fn assertion_name(assertion: &TestAssertion) -> String {
    match assertion {
        TestAssertion::Status { op, value } => format!("status {op} {value}"),
        TestAssertion::Json { path, op, value } => match value {
            Some(v) => format!("json {path} {op} {}", node_to_string(v)),
            None => format!("json {path} {op}"),
        },
        TestAssertion::Header { name, op, value } => match value {
            Some(v) => format!("header {name} {op} {v}"),
            None => format!("header {name} {op}"),
        },
        TestAssertion::Duration { op, value } => format!("duration {op} {value}ms"),
    }
}

pub fn evaluate_tests(
    tests: &[TestAssertion],
    status: u16,
    headers: &[(String, String)],
    body: &str,
    duration_ms: u128,
) -> Vec<TestResult> {
    let mut parsed_body: Option<Result<Value, String>> = None;
    tests
        .iter()
        .map(|assertion| {
            let outcome = match assertion {
                TestAssertion::Status { op, value } => check_status(*op, *value, status),
                TestAssertion::Duration { op, value } => check_duration(*op, *value, duration_ms),
                TestAssertion::Header { name, op, value } => {
                    check_header(name, *op, value.as_deref(), headers)
                }
                TestAssertion::Json { path, op, value } => {
                    let json = parsed_body.get_or_insert_with(|| {
                        serde_json::from_str::<Value>(body)
                            .map_err(|e| format!("response body is not JSON: {e}"))
                    });
                    match json {
                        Ok(json) => check_json(path, *op, value.as_ref(), json),
                        Err(e) => Err(e.clone()),
                    }
                }
            };
            TestResult {
                name: assertion_name(assertion),
                passed: outcome.is_ok(),
                detail: outcome.err(),
            }
        })
        .collect()
}

fn check_status(op: StatusOp, expected: u16, actual: u16) -> Result<(), String> {
    let ok = match op {
        StatusOp::Eq => actual == expected,
        StatusOp::Ne => actual != expected,
        StatusOp::Lt => actual < expected,
        StatusOp::Gt => actual > expected,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("status was {actual}"))
    }
}

fn check_duration(op: DurationOp, expected: u128, actual: u128) -> Result<(), String> {
    let ok = match op {
        DurationOp::Lt => actual < expected,
        DurationOp::Gt => actual > expected,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("took {actual}ms"))
    }
}

fn check_header(
    name: &str,
    op: HeaderOp,
    expected: Option<&str>,
    headers: &[(String, String)],
) -> Result<(), String> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .collect();
    match op {
        HeaderOp::Exists => {
            if values.is_empty() {
                return Err(format!("header {name} is absent"));
            }
            Ok(())
        }
        HeaderOp::Absent => {
            if values.is_empty() {
                Ok(())
            } else {
                Err(format!("header {name} is {}", values.join(", ")))
            }
        }
        HeaderOp::Eq | HeaderOp::Contains | HeaderOp::Matches => {
            let expected = expected.ok_or_else(|| format!("header {op} needs a value"))?;
            if values.is_empty() {
                return Err(format!("header {name} is absent"));
            }
            let ok = match op {
                HeaderOp::Eq => values.contains(&expected),
                HeaderOp::Contains => values.iter().any(|v| v.contains(expected)),
                _ => {
                    let re = regex::Regex::new(expected)
                        .map_err(|e| format!("invalid regex {expected:?}: {e}"))?;
                    values.iter().any(|v| re.is_match(v))
                }
            };
            if ok {
                Ok(())
            } else {
                Err(format!("header {name} is {}", values.join(", ")))
            }
        }
    }
}

fn check_json(
    path: &str,
    op: JsonOp,
    expected: Option<&Value>,
    json: &Value,
) -> Result<(), String> {
    let parsed = JsonPath::parse(path).map_err(|e| format!("invalid JSONPath {path:?}: {e}"))?;
    let nodes = parsed.query(json);
    let node = nodes.first();
    match op {
        JsonOp::Exists => {
            return match node {
                Some(_) => Ok(()),
                None => Err(format!("{path} matched nothing")),
            }
        }
        JsonOp::Absent => {
            return match node {
                None => Ok(()),
                Some(n) => Err(format!("{path} is {n}")),
            }
        }
        _ => {}
    }
    let node = node.ok_or_else(|| format!("{path} matched nothing"))?;
    let expected = expected.ok_or_else(|| format!("json {op} needs a value"))?;
    let ok = match op {
        JsonOp::Eq => json_eq(node, expected),
        JsonOp::Ne => !json_eq(node, expected),
        JsonOp::Lt | JsonOp::Gt => {
            let a = number(node).ok_or_else(|| format!("{path} is not a number: {node}"))?;
            let b = number(expected)
                .ok_or_else(|| format!("json {op} needs a numeric value, got {expected}"))?;
            if op == JsonOp::Lt {
                a < b
            } else {
                a > b
            }
        }
        JsonOp::Len => {
            let want = number(expected)
                .ok_or_else(|| format!("json len needs a numeric value, got {expected}"))?;
            let got = match node {
                Value::Array(a) => a.len(),
                Value::Object(o) => o.len(),
                Value::String(s) => s.chars().count(),
                other => return Err(format!("{path} has no length: {other}")),
            };
            got as f64 == want
        }
        JsonOp::Contains => {
            let needle = expected;
            match node {
                Value::String(s) => match needle {
                    Value::String(n) => s.contains(n.as_str()),
                    other => s.contains(&other.to_string()),
                },
                Value::Array(items) => items.iter().any(|item| json_eq(item, needle)),
                Value::Object(map) => match needle {
                    Value::String(key) => map.contains_key(key.as_str()),
                    other => return Err(format!("{path} is an object, {other} is not a key")),
                },
                other => return Err(format!("{path} cannot contain anything: {other}")),
            }
        }
        JsonOp::Matches => {
            let pattern = expected
                .as_str()
                .ok_or_else(|| format!("json matches needs a string pattern, got {expected}"))?;
            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("invalid regex {pattern:?}: {e}"))?;
            re.is_match(&node_to_string(node))
        }
        JsonOp::Exists | JsonOp::Absent => unreachable!(),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{path} is {node}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BODY: &str =
        r#"{"id": 7, "name": "nova", "tags": ["a", "b"], "nested": {"ok": true}, "empty": null}"#;

    fn headers() -> Vec<(String, String)> {
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Trace".to_string(), "abc-123".to_string()),
        ]
    }

    fn run(assertion: TestAssertion) -> TestResult {
        evaluate_tests(&[assertion], 200, &headers(), BODY, 120)
            .pop()
            .unwrap()
    }

    fn status(op: StatusOp, value: u16) -> TestAssertion {
        TestAssertion::Status { op, value }
    }

    fn jsonp(path: &str, op: JsonOp, value: Option<Value>) -> TestAssertion {
        TestAssertion::Json {
            path: path.to_string(),
            op,
            value,
        }
    }

    fn header(name: &str, op: HeaderOp, value: Option<&str>) -> TestAssertion {
        TestAssertion::Header {
            name: name.to_string(),
            op,
            value: value.map(String::from),
        }
    }

    #[test]
    fn status_ops() {
        assert!(run(status(StatusOp::Eq, 200)).passed);
        assert!(!run(status(StatusOp::Eq, 201)).passed);
        assert!(run(status(StatusOp::Ne, 500)).passed);
        assert!(run(status(StatusOp::Lt, 300)).passed);
        assert!(run(status(StatusOp::Gt, 199)).passed);
        assert!(!run(status(StatusOp::Gt, 200)).passed);
    }

    #[test]
    fn status_failure_names_and_details() {
        let result = run(status(StatusOp::Eq, 404));
        assert_eq!(result.name, "status eq 404");
        assert_eq!(result.detail.as_deref(), Some("status was 200"));
    }

    #[test]
    fn passing_result_has_no_detail() {
        assert_eq!(run(status(StatusOp::Eq, 200)).detail, None);
    }

    #[test]
    fn duration_ops() {
        assert!(
            run(TestAssertion::Duration {
                op: DurationOp::Lt,
                value: 500
            })
            .passed
        );
        let slow = run(TestAssertion::Duration {
            op: DurationOp::Gt,
            value: 500,
        });
        assert!(!slow.passed);
        assert_eq!(slow.name, "duration gt 500ms");
        assert_eq!(slow.detail.as_deref(), Some("took 120ms"));
    }

    #[test]
    fn json_exists_and_absent() {
        assert!(run(jsonp("$.id", JsonOp::Exists, None)).passed);
        assert!(!run(jsonp("$.nope", JsonOp::Exists, None)).passed);
        assert!(run(jsonp("$.nope", JsonOp::Absent, None)).passed);
        assert!(!run(jsonp("$.id", JsonOp::Absent, None)).passed);
        assert_eq!(
            run(jsonp("$.id", JsonOp::Exists, None)).name,
            "json $.id exists"
        );
    }

    #[test]
    fn json_eq_and_ne_compare_numbers_across_int_float() {
        assert!(run(jsonp("$.id", JsonOp::Eq, Some(json!(7)))).passed);
        assert!(run(jsonp("$.id", JsonOp::Eq, Some(json!(7.0)))).passed);
        assert!(run(jsonp("$.name", JsonOp::Eq, Some(json!("nova")))).passed);
        assert!(run(jsonp("$.name", JsonOp::Ne, Some(json!("other")))).passed);
        assert!(!run(jsonp("$.name", JsonOp::Ne, Some(json!("nova")))).passed);
        assert!(run(jsonp("$.nested", JsonOp::Eq, Some(json!({"ok": true})))).passed);
    }

    #[test]
    fn json_numeric_ops() {
        assert!(run(jsonp("$.id", JsonOp::Lt, Some(json!(10)))).passed);
        assert!(run(jsonp("$.id", JsonOp::Gt, Some(json!(1)))).passed);
        let bad = run(jsonp("$.name", JsonOp::Lt, Some(json!(1))));
        assert!(!bad.passed);
        assert!(bad.detail.unwrap().contains("is not a number"));
    }

    #[test]
    fn json_len_over_array_string_object() {
        assert!(run(jsonp("$.tags", JsonOp::Len, Some(json!(2)))).passed);
        assert!(run(jsonp("$.name", JsonOp::Len, Some(json!(4)))).passed);
        assert!(run(jsonp("$.nested", JsonOp::Len, Some(json!(1)))).passed);
        assert!(!run(jsonp("$.tags", JsonOp::Len, Some(json!(3)))).passed);
        assert!(run(jsonp("$.id", JsonOp::Len, Some(json!(1))))
            .detail
            .unwrap()
            .contains("has no length"));
    }

    #[test]
    fn json_contains_over_string_array_object() {
        assert!(run(jsonp("$.name", JsonOp::Contains, Some(json!("ov")))).passed);
        assert!(run(jsonp("$.tags", JsonOp::Contains, Some(json!("b")))).passed);
        assert!(!run(jsonp("$.tags", JsonOp::Contains, Some(json!("z")))).passed);
        assert!(run(jsonp("$.nested", JsonOp::Contains, Some(json!("ok")))).passed);
    }

    #[test]
    fn json_matches_regex() {
        assert!(run(jsonp("$.name", JsonOp::Matches, Some(json!("^no.a$")))).passed);
        assert!(!run(jsonp("$.name", JsonOp::Matches, Some(json!("^zz")))).passed);
        let bad = run(jsonp("$.name", JsonOp::Matches, Some(json!("[unclosed"))));
        assert!(bad.detail.unwrap().contains("invalid regex"));
    }

    #[test]
    fn json_ops_without_value_fail_loud() {
        let result = run(jsonp("$.id", JsonOp::Eq, None));
        assert!(!result.passed);
        assert_eq!(result.detail.as_deref(), Some("json eq needs a value"));
    }

    #[test]
    fn json_assertion_on_non_json_body_fails_loud() {
        let results = evaluate_tests(
            &[
                jsonp("$.id", JsonOp::Exists, None),
                status(StatusOp::Eq, 200),
            ],
            200,
            &headers(),
            "<html/>",
            5,
        );
        assert!(!results[0].passed);
        assert!(results[0].detail.as_ref().unwrap().contains("not JSON"));
        assert!(results[1].passed);
    }

    #[test]
    fn invalid_jsonpath_fails_loud() {
        let result = run(jsonp("not a path", JsonOp::Exists, None));
        assert!(!result.passed);
        assert!(result.detail.unwrap().contains("invalid JSONPath"));
    }

    #[test]
    fn header_ops_are_case_insensitive() {
        assert!(run(header("content-type", HeaderOp::Exists, None)).passed);
        assert!(run(header("X-Missing", HeaderOp::Absent, None)).passed);
        assert!(!run(header("X-Trace", HeaderOp::Absent, None)).passed);
        assert!(
            run(header(
                "Content-Type",
                HeaderOp::Eq,
                Some("application/json")
            ))
            .passed
        );
        assert!(run(header("Content-Type", HeaderOp::Contains, Some("json"))).passed);
        assert!(run(header("X-Trace", HeaderOp::Matches, Some(r"^abc-\d+$"))).passed);
        assert!(!run(header("X-Trace", HeaderOp::Matches, Some(r"^\d+$"))).passed);
    }

    #[test]
    fn header_value_ops_report_absence_and_missing_value() {
        let absent = run(header("X-Missing", HeaderOp::Eq, Some("x")));
        assert_eq!(absent.detail.as_deref(), Some("header X-Missing is absent"));
        let no_value = run(header("X-Trace", HeaderOp::Eq, None));
        assert_eq!(no_value.detail.as_deref(), Some("header eq needs a value"));
        assert_eq!(no_value.name, "header X-Trace eq");
    }

    #[test]
    fn header_matches_any_of_repeated_values() {
        let repeated = vec![
            ("Set-Cookie".to_string(), "a=1".to_string()),
            ("Set-Cookie".to_string(), "b=2".to_string()),
        ];
        let results = evaluate_tests(
            &[header("set-cookie", HeaderOp::Contains, Some("b="))],
            200,
            &repeated,
            "{}",
            1,
        );
        assert!(results[0].passed);
    }

    #[test]
    fn evaluate_preserves_order_and_count() {
        let results = evaluate_tests(
            &[
                status(StatusOp::Eq, 200),
                jsonp("$.id", JsonOp::Eq, Some(json!(7))),
                header("X-Trace", HeaderOp::Exists, None),
            ],
            200,
            &headers(),
            BODY,
            1,
        );
        assert_eq!(results.len(), 3);
        assert_eq!(
            results.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["status eq 200", "json $.id eq 7", "header X-Trace exists"]
        );
        assert!(results.iter().all(|r| r.passed));
    }

    #[test]
    fn capture_status_header_and_body() {
        assert_eq!(
            resolve_capture("status", 201, &headers(), BODY).unwrap(),
            Some("201".to_string())
        );
        assert_eq!(
            resolve_capture("header.content-type", 200, &headers(), BODY).unwrap(),
            Some("application/json".to_string())
        );
        assert_eq!(
            resolve_capture("body.$.name", 200, &headers(), BODY).unwrap(),
            Some("nova".to_string())
        );
        assert_eq!(
            resolve_capture("body.$.id", 200, &headers(), BODY).unwrap(),
            Some("7".to_string())
        );
        assert_eq!(
            resolve_capture("body.$.nested", 200, &headers(), BODY).unwrap(),
            Some("{\"ok\":true}".to_string())
        );
        assert_eq!(
            resolve_capture("body.$.tags[1]", 200, &headers(), BODY).unwrap(),
            Some("b".to_string())
        );
    }

    #[test]
    fn capture_missing_target_is_none_not_error() {
        assert_eq!(
            resolve_capture("header.X-Missing", 200, &headers(), BODY).unwrap(),
            None
        );
        assert_eq!(
            resolve_capture("body.$.nope", 200, &headers(), BODY).unwrap(),
            None
        );
    }

    #[test]
    fn capture_source_grammar_is_validated_loudly() {
        assert!(resolve_capture("cookie.session", 200, &[], BODY)
            .unwrap_err()
            .to_string()
            .contains("invalid capture source"));
        assert!(resolve_capture("body.name", 200, &[], BODY)
            .unwrap_err()
            .to_string()
            .contains("must start with $"));
        assert!(resolve_capture("header.", 200, &[], BODY)
            .unwrap_err()
            .to_string()
            .contains("empty header name"));
        assert!(resolve_capture("body.$.[", 200, &[], BODY)
            .unwrap_err()
            .to_string()
            .contains("invalid JSONPath"));
    }

    #[test]
    fn capture_on_non_json_body_fails_loud() {
        let err = resolve_capture("body.$.id", 200, &[], "<html/>")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not JSON"));
    }

    #[test]
    fn validate_capture_rejects_bad_target() {
        let capture = |from: &str, into: &str| Capture {
            from: from.to_string(),
            into: into.to_string(),
            scope: CaptureScope::Run,
        };
        validate_capture(&capture("status", "token")).unwrap();
        assert!(validate_capture(&capture("status", "")).is_err());
        assert!(validate_capture(&capture("status", "a b")).is_err());
        assert!(validate_capture(&capture("nope", "token")).is_err());
    }

    #[test]
    fn assertions_roundtrip_camel_case_json() {
        let assertion: TestAssertion =
            serde_json::from_value(json!({"kind": "json", "path": "$.id", "op": "exists"}))
                .unwrap();
        assert_eq!(assertion, jsonp("$.id", JsonOp::Exists, None));
        assert_eq!(
            serde_json::to_value(&assertion).unwrap(),
            json!({"kind": "json", "path": "$.id", "op": "exists", "value": null})
        );
        let duration: TestAssertion =
            serde_json::from_value(json!({"kind": "duration", "op": "lt", "value": 800})).unwrap();
        assert_eq!(
            duration,
            TestAssertion::Duration {
                op: DurationOp::Lt,
                value: 800
            }
        );
    }

    #[test]
    fn unknown_op_fails_loud() {
        let err = serde_json::from_value::<TestAssertion>(
            json!({"kind": "status", "op": "approx", "value": 200}),
        )
        .unwrap_err()
        .to_string();
        assert!(err.to_string().contains("approx"));
    }

    #[test]
    fn assertions_roundtrip_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Holder {
            tests: Vec<TestAssertion>,
            captures: Vec<Capture>,
            scripts: Scripts,
        }
        let holder = Holder {
            tests: vec![
                status(StatusOp::Eq, 200),
                jsonp("$.id", JsonOp::Eq, Some(json!(7))),
                header("X-Trace", HeaderOp::Exists, None),
                TestAssertion::Duration {
                    op: DurationOp::Lt,
                    value: 900,
                },
            ],
            captures: vec![Capture {
                from: "body.$.token".to_string(),
                into: "token".to_string(),
                scope: CaptureScope::Persist,
            }],
            scripts: Scripts {
                pre: Some("console.log(1)".to_string()),
                post: None,
            },
        };
        let raw = toml::to_string_pretty(&holder).unwrap();
        assert_eq!(toml::from_str::<Holder>(&raw).unwrap(), holder);
    }
}
