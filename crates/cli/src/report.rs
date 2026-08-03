use crate::style::Style;
use mandalo_core::grpc::GrpcResponse;
use mandalo_core::request::ResponseData;
use mandalo_core::runner::{CaptureOutcome, RunReport, StepResult};
use mandalo_core::{CoreError, CoreResult};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum Reporter {
    Pretty,
    Json,
    Junit,
}

/// Reporters for the commands that print data rather than a test run. `junit` is
/// meaningless outside `run`, so it is deliberately absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum DataReporter {
    Pretty,
    Json,
}

pub fn to_json<T: Serialize>(value: &T) -> CoreResult<String> {
    serde_json::to_string_pretty(value)
        .map_err(|e| CoreError::Parse(format!("cannot render the output as JSON: {e}")))
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JsonTestKind {
    /// A declared `[[tests]]` assertion.
    Assertion,
    /// A `test(...)` call from a pre/post script.
    Script,
}

/// One test case, with an identifier that stays stable across runs so a consumer can
/// key its own UI off it instead of the human-readable name (names are not unique).
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JsonTest {
    pub id: String,
    pub name: String,
    pub kind: JsonTestKind,
    pub passed: bool,
    pub detail: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JsonRequest {
    /// Collection-relative POSIX path including the `.toml` extension, e.g. `auth/login.toml`.
    pub path: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub response: Option<ResponseData>,
    pub grpc: Option<GrpcResponse>,
    pub tests: Vec<JsonTest>,
    pub captures: Vec<CaptureOutcome>,
    pub logs: Vec<String>,
    pub passed: bool,
    pub duration_ms: u128,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JsonRun {
    pub collection: String,
    pub env: Option<String>,
    pub total: usize,
    /// How many requests passed — a count, matching `total` and `failed`.
    pub passed: usize,
    pub failed: usize,
    pub duration_ms: u128,
    pub requests: Vec<JsonRequest>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JsonSend {
    pub collection: String,
    pub env: Option<String>,
    #[serde(flatten)]
    pub request: JsonRequest,
}

fn json_tests(step: &StepResult) -> Vec<JsonTest> {
    let assertions = step.tests.iter().enumerate().map(|(i, test)| JsonTest {
        id: format!("test:{i}"),
        name: test.name.clone(),
        kind: JsonTestKind::Assertion,
        passed: test.passed,
        detail: test.detail.clone(),
    });
    let scripted = step
        .script_tests
        .iter()
        .enumerate()
        .map(|(i, test)| JsonTest {
            id: format!("script:{i}"),
            name: test.name.clone(),
            kind: JsonTestKind::Script,
            passed: test.passed,
            detail: test.error.clone(),
        });
    assertions.chain(scripted).collect()
}

pub fn json_request(step: &StepResult) -> JsonRequest {
    JsonRequest {
        path: step.path.clone(),
        name: step.request_name.clone(),
        method: step.method.clone(),
        url: step.url.clone(),
        response: step.response.clone(),
        grpc: step.grpc.clone(),
        tests: json_tests(step),
        captures: step.captures.clone(),
        logs: step.logs.clone(),
        passed: step.passed,
        duration_ms: step.duration_ms,
        error: step.error.clone(),
        error_code: step.error_code.clone(),
    }
}

pub fn json_run(report: &RunReport) -> JsonRun {
    JsonRun {
        collection: report.collection.clone(),
        env: report.environment.clone(),
        total: report.total,
        passed: report.total.saturating_sub(report.failed),
        failed: report.failed,
        duration_ms: report.duration_ms,
        requests: report.steps.iter().map(json_request).collect(),
    }
}

pub fn render(reporter: Reporter, report: &RunReport, style: &Style) -> CoreResult<String> {
    match reporter {
        Reporter::Pretty => Ok(pretty(report, style)),
        Reporter::Json => to_json(&json_run(report)),
        Reporter::Junit => Ok(junit(report)),
    }
}

fn pretty(report: &RunReport, style: &Style) -> String {
    let mut out = String::new();
    for step in &report.steps {
        let failures = step.failures();
        let marker = if step.passed {
            style.pass("PASS")
        } else {
            style.fail("FAIL")
        };
        out.push_str(&format!(
            "{marker}  {}  {}\n",
            step.request_name,
            style.dim(&format!("{} · {}ms", step.path, step.duration_ms))
        ));
        for test in &step.tests {
            out.push_str(&format!(
                "      {} {}\n",
                if test.passed {
                    style.pass("·")
                } else {
                    style.fail("×")
                },
                test.name
            ));
        }
        for test in &step.script_tests {
            out.push_str(&format!(
                "      {} {}\n",
                if test.passed {
                    style.pass("·")
                } else {
                    style.fail("×")
                },
                test.name
            ));
        }
        for failure in failures {
            out.push_str(&format!("      {}\n", style.fail(&failure)));
        }
    }
    let summary = format!(
        "{} of {} requests failed in {}ms",
        report.failed, report.total, report.duration_ms
    );
    out.push('\n');
    out.push_str(&if report.passed {
        style.pass(&format!(
            "all {} requests passed in {}ms",
            report.total, report.duration_ms
        ))
    } else {
        style.fail(&summary)
    });
    out.push('\n');
    out
}

fn escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

fn case_count(step: &StepResult) -> usize {
    step.tests.len() + step.script_tests.len()
}

fn junit(report: &RunReport) -> String {
    let cases: usize = report
        .steps
        .iter()
        .map(|s| case_count(s).max(1))
        .sum::<usize>();
    let failures: usize = report
        .steps
        .iter()
        .map(|s| {
            let counted = s.tests.iter().filter(|t| !t.passed).count()
                + s.script_tests.iter().filter(|t| !t.passed).count();
            if counted == 0 && !s.passed {
                1
            } else {
                counted
            }
        })
        .sum();
    let seconds = |ms: u128| format!("{:.3}", ms as f64 / 1000.0);

    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"mandalo\" tests=\"{cases}\" failures=\"{failures}\" time=\"{}\">\n",
        seconds(report.duration_ms)
    ));
    out.push_str(&format!(
        "  <testsuite name=\"{}\" tests=\"{cases}\" failures=\"{failures}\" time=\"{}\">\n",
        escape(&report.collection),
        seconds(report.duration_ms)
    ));
    for step in &report.steps {
        let classname = escape(&format!("{}.{}", report.collection, step.path));
        if case_count(step) == 0 {
            out.push_str(&format!(
                "    <testcase classname=\"{classname}\" name=\"{}\" time=\"{}\">",
                escape(&step.request_name),
                seconds(step.duration_ms)
            ));
            if let Some(error) = &step.error {
                out.push_str(&format!(
                    "\n      <failure message=\"{}\">{}</failure>\n    ",
                    escape(error),
                    escape(error)
                ));
            }
            out.push_str("</testcase>\n");
            continue;
        }
        for test in &step.tests {
            out.push_str(&testcase(
                &classname,
                &format!("{} · {}", step.request_name, test.name),
                test.passed,
                test.detail.as_deref(),
                &seconds(step.duration_ms),
            ));
        }
        for test in &step.script_tests {
            out.push_str(&testcase(
                &classname,
                &format!("{} · {}", step.request_name, test.name),
                test.passed,
                test.error.as_deref(),
                &seconds(step.duration_ms),
            ));
        }
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

fn testcase(classname: &str, name: &str, passed: bool, detail: Option<&str>, time: &str) -> String {
    if passed {
        return format!(
            "    <testcase classname=\"{classname}\" name=\"{}\" time=\"{time}\"></testcase>\n",
            escape(name)
        );
    }
    let message = escape(detail.unwrap_or("assertion failed"));
    format!(
        "    <testcase classname=\"{classname}\" name=\"{}\" time=\"{time}\">\n      <failure message=\"{message}\">{message}</failure>\n    </testcase>\n",
        escape(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandalo_core::assertions::{CaptureScope, TestResult};
    use mandalo_core::script::ScriptTest;
    use std::collections::BTreeMap;

    fn step(name: &str, passed: bool, tests: Vec<TestResult>) -> StepResult {
        StepResult {
            request_name: name.to_string(),
            path: format!("{name}.toml"),
            method: "GET".to_string(),
            url: format!("https://api.example.com/{name}"),
            response: None,
            grpc: None,
            tests,
            script_tests: Vec::new(),
            logs: Vec::new(),
            captured: BTreeMap::new(),
            captures: Vec::new(),
            unbound_secrets: Vec::new(),
            var_sets: BTreeMap::new(),
            var_unsets: Vec::new(),
            secret_var_sets: Vec::new(),
            passed,
            duration_ms: 12,
            error: None,
            error_code: None,
        }
    }

    fn test(name: &str, passed: bool, detail: Option<&str>) -> TestResult {
        TestResult {
            name: name.to_string(),
            passed,
            detail: detail.map(String::from),
        }
    }

    fn report(steps: Vec<StepResult>) -> RunReport {
        let failed = steps.iter().filter(|s| !s.passed).count();
        RunReport {
            collection: "orders".to_string(),
            environment: Some("staging".to_string()),
            total: steps.len(),
            failed,
            passed: failed == 0,
            duration_ms: 1234,
            steps,
        }
    }

    #[test]
    fn junit_has_a_testsuite_with_counted_cases_and_failures() {
        let xml = junit(&report(vec![
            step("Login", true, vec![test("status eq 200", true, None)]),
            step(
                "Orders",
                false,
                vec![
                    test("status eq 200", true, None),
                    test("json $.total gt 0", false, Some("$.total is 0")),
                ],
            ),
        ]));
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(xml.contains("<testsuites name=\"mandalo\" tests=\"3\" failures=\"1\""));
        assert!(xml.contains("<testsuite name=\"orders\" tests=\"3\" failures=\"1\""));
        assert!(xml.contains("classname=\"orders.Orders.toml\""));
        assert!(xml.contains("name=\"Orders · json $.total gt 0\""));
        assert!(xml.contains("<failure message=\"$.total is 0\">$.total is 0</failure>"));
        assert!(xml.trim_end().ends_with("</testsuites>"));
        assert_eq!(xml.matches("<testcase").count(), 3);
    }

    #[test]
    fn junit_escapes_xml_metacharacters() {
        let xml = junit(&report(vec![step(
            "Search <a> & \"b\"",
            false,
            vec![test("json $.q eq <x>", false, Some("$.q is \"a & b\""))],
        )]));
        assert!(xml.contains("Search &lt;a&gt; &amp; &quot;b&quot;"));
        assert!(!xml.contains("<a>"));
        assert!(xml.contains("$.q is &quot;a &amp; b&quot;"));
    }

    #[test]
    fn a_transport_failure_still_produces_one_failing_case() {
        let mut broken = step("Login", false, Vec::new());
        broken.error = Some("cannot resolve api.example.com".to_string());
        let xml = junit(&report(vec![broken]));
        assert!(xml.contains("tests=\"1\" failures=\"1\""));
        assert!(xml.contains("<failure message=\"cannot resolve api.example.com\""));
    }

    #[test]
    fn pretty_marks_each_step_and_summarises() {
        let text = pretty(
            &report(vec![
                step("Login", true, vec![test("status eq 200", true, None)]),
                step(
                    "Orders",
                    false,
                    vec![test("status eq 200", false, Some("status was 500"))],
                ),
            ]),
            &Style::plain(),
        );
        assert!(text.contains("PASS  Login"));
        assert!(text.contains("FAIL  Orders"));
        assert!(text.contains("status was 500"));
        assert!(text.contains("1 of 2 requests failed"));
    }

    fn json_of(report: &RunReport) -> serde_json::Value {
        serde_json::from_str(&render(Reporter::Json, report, &Style::plain()).unwrap()).unwrap()
    }

    #[test]
    fn json_reporter_emits_the_documented_run_envelope() {
        let value = json_of(&report(vec![
            step("Login", true, vec![test("status eq 200", true, None)]),
            step(
                "Orders",
                false,
                vec![test("json $.total gt 0", false, Some("$.total is 0"))],
            ),
        ]));

        assert_eq!(
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "collection",
                "durationMs",
                "env",
                "failed",
                "passed",
                "requests",
                "total"
            ],
            "the run envelope has exactly these keys"
        );
        assert_eq!(value["collection"], "orders");
        assert_eq!(value["env"], "staging");
        assert_eq!(value["total"], 2);
        assert_eq!(
            value["passed"], 1,
            "top-level passed is a count, not a flag"
        );
        assert_eq!(value["failed"], 1);
        assert_eq!(value["durationMs"], 1234);
        assert_eq!(value["requests"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_json_request_carries_its_path_identity_and_stable_test_ids() {
        let value = json_of(&report(vec![step(
            "Login",
            false,
            vec![
                test("status eq 200", true, None),
                test("status eq 200", false, Some("status was 500")),
            ],
        )]));
        let request = &value["requests"][0];

        assert_eq!(
            request.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "captures",
                "durationMs",
                "error",
                "errorCode",
                "grpc",
                "logs",
                "method",
                "name",
                "passed",
                "path",
                "response",
                "tests",
                "url"
            ],
            "a request outcome has exactly these keys"
        );
        assert_eq!(request["path"], "Login.toml");
        assert_eq!(request["name"], "Login");
        assert_eq!(request["method"], "GET");
        assert_eq!(request["url"], "https://api.example.com/Login");
        assert_eq!(request["passed"], false);
        assert_eq!(request["response"], serde_json::Value::Null);

        let tests = request["tests"].as_array().unwrap();
        assert_eq!(tests[0]["id"], "test:0");
        assert_eq!(tests[1]["id"], "test:1");
        assert_eq!(
            tests[0]["name"], tests[1]["name"],
            "the fixture pins that duplicate names still get distinct ids"
        );
        assert_eq!(tests[0]["kind"], "assertion");
        assert_eq!(tests[0]["passed"], true);
        assert_eq!(tests[0]["detail"], serde_json::Value::Null);
        assert_eq!(tests[1]["passed"], false);
        assert_eq!(tests[1]["detail"], "status was 500");
    }

    #[test]
    fn script_tests_join_the_assertions_under_their_own_id_space() {
        let mut scripted = step("Login", false, vec![test("status eq 200", true, None)]);
        scripted.script_tests = vec![
            ScriptTest {
                name: "token looks like a jwt".to_string(),
                passed: false,
                error: Some("expected 3 segments".to_string()),
            },
            ScriptTest {
                name: "body is small".to_string(),
                passed: true,
                error: None,
            },
        ];
        let value = json_of(&report(vec![scripted]));
        let tests = value["requests"][0]["tests"].as_array().unwrap();

        assert_eq!(tests.len(), 3);
        assert_eq!(tests[0]["id"], "test:0");
        assert_eq!(tests[1]["id"], "script:0");
        assert_eq!(tests[1]["kind"], "script");
        assert_eq!(tests[1]["detail"], "expected 3 segments");
        assert_eq!(tests[2]["id"], "script:1");
    }

    #[test]
    fn a_transport_failure_reports_the_error_with_no_response() {
        let mut broken = step("Login", false, Vec::new());
        broken.error = Some("cannot resolve api.example.com".to_string());
        broken.error_code = Some("E_NETWORK".to_string());
        let request = &json_of(&report(vec![broken]))["requests"][0];

        assert_eq!(request["response"], serde_json::Value::Null);
        assert_eq!(request["error"], "cannot resolve api.example.com");
        assert_eq!(request["errorCode"], "E_NETWORK");
        assert_eq!(request["tests"].as_array().unwrap().len(), 0);
        assert_eq!(request["passed"], false);
    }

    #[test]
    fn captures_carry_the_value_they_resolved_to() {
        let mut captured = step("Login", true, Vec::new());
        captured.captures = vec![CaptureOutcome {
            from: "body.$.token".to_string(),
            into: "token".to_string(),
            value: "abc123".to_string(),
            scope: CaptureScope::Run,
        }];
        let captures = &json_of(&report(vec![captured]))["requests"][0]["captures"];

        assert_eq!(
            captures[0],
            serde_json::json!({
                "from": "body.$.token",
                "into": "token",
                "value": "abc123",
                "scope": "run"
            })
        );
    }

    #[test]
    fn an_empty_run_still_emits_the_envelope() {
        let value = json_of(&report(Vec::new()));
        assert_eq!(value["total"], 0);
        assert_eq!(value["passed"], 0);
        assert_eq!(value["failed"], 0);
        assert_eq!(value["requests"].as_array().unwrap().len(), 0);
    }
}
