use mandalo_core::collection::{self, SavedRequest};
use mandalo_core::postman;
use mandalo_core::script::{
    self, Limits, ScriptContext, ScriptOutcome, ScriptRequest, ScriptResponse,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Pre,
    Post,
    NotFound,
    Empty,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    Pass,
    Fail,
    Loud,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Status {
    Works,
    FailsLoud,
    Confusing,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Works => "works",
            Status::FailsLoud => "fails loud",
            Status::Confusing => "CONFUSING",
        }
    }
}

struct Idiom {
    name: &'static str,
    phase: Phase,
    expect: Expect,
    source: &'static str,
}

const JSON_BODY: &str = r#"{"id":7,"token":"t0k","count":3,"ok":true,"data":[{"id":1},{"id":2},{"id":3}],"user":{"name":"nova","email":"nova@x.dev"}}"#;

fn context(phase: Phase) -> ScriptContext {
    let mut vars = BTreeMap::new();
    vars.insert("base".to_string(), "https://api.x.dev".to_string());
    vars.insert("token".to_string(), "t0k".to_string());
    let request = ScriptRequest {
        method: "GET".to_string(),
        url: "https://api.x.dev/users/7".to_string(),
        headers: vec![("Accept".to_string(), "application/json".to_string())],
        body: None,
    };
    let response = match phase {
        Phase::Pre => None,
        Phase::Post => Some(ScriptResponse {
            status: 200,
            status_text: "OK".to_string(),
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("X-Request-Id".to_string(), "abc-123".to_string()),
            ],
            body: JSON_BODY.to_string(),
            duration_ms: 42,
        }),
        Phase::NotFound => Some(ScriptResponse {
            status: 404,
            status_text: "Not Found".to_string(),
            headers: vec![("Content-Type".to_string(), "text/html".to_string())],
            body: "<html>nope</html>".to_string(),
            duration_ms: 12,
        }),
        Phase::Empty => Some(ScriptResponse {
            status: 204,
            status_text: "No Content".to_string(),
            headers: vec![],
            body: String::new(),
            duration_ms: 3,
        }),
    };
    ScriptContext {
        vars,
        request_name: "Get user".to_string(),
        request,
        response,
    }
}

fn named_limitation(message: &str) -> bool {
    message.contains("not available in Mándalo")
        || message.contains("not supported in Mándalo")
        || message.contains("Mándalo does not")
}

fn classify(idiom: &Idiom) -> (Status, String) {
    let outcome = script::run_script(idiom.source, context(idiom.phase), Limits::default());
    match (idiom.expect, outcome) {
        (Expect::Loud, Ok(out)) => (
            Status::Confusing,
            format!("expected a named limitation error, script ran silently: {out:?}"),
        ),
        (Expect::Loud, Err(e)) => {
            let message = e.to_string();
            if named_limitation(&message) {
                (Status::FailsLoud, message)
            } else {
                (Status::Confusing, message)
            }
        }
        (_, Err(e)) => {
            let message = e.to_string();
            if named_limitation(&message) {
                (Status::FailsLoud, message)
            } else {
                (Status::Confusing, message)
            }
        }
        (want, Ok(out)) => verdict(want, out),
    }
}

fn verdict(want: Expect, out: ScriptOutcome) -> (Status, String) {
    let Some(test) = out.tests.first() else {
        return (
            Status::Confusing,
            "the snippet recorded no pm.test result at all".to_string(),
        );
    };
    let detail = test.error.clone().unwrap_or_else(|| "ok".to_string());
    match (want, test.passed) {
        (Expect::Pass, true) => (Status::Works, detail),
        (Expect::Pass, false) => (
            Status::Confusing,
            format!("assertion that Postman passes failed here: {detail}"),
        ),
        (Expect::Fail, false) => (Status::Works, detail),
        (Expect::Fail, true) => (
            Status::Confusing,
            "assertion silently passed — the check is a no-op".to_string(),
        ),
        (Expect::Loud, _) => unreachable!(),
    }
}

const IDIOMS: &[Idiom] = &[
    Idiom {
        name: "pm.test + pm.response.to.have.status(200)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("Status code is 200", function () { pm.response.to.have.status(200); });"#,
    },
    Idiom {
        name: "pm.response.to.have.status(wrong) reports the mismatch",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.have.status(404); });"#,
    },
    Idiom {
        name: "pm.response.to.have.status(\"OK\")",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.status("OK"); });"#,
    },
    Idiom {
        name: "pm.expect(pm.response.text()).to.include(...)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("Body matches string", function () { pm.expect(pm.response.text()).to.include("nova"); });"#,
    },
    Idiom {
        name: "pm.response.to.be.ok",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.ok; });"#,
    },
    Idiom {
        name: "pm.response.to.be.ok on a 404 must fail",
        phase: Phase::NotFound,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.ok; });"#,
    },
    Idiom {
        name: "pm.response.to.be.json",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.json; });"#,
    },
    Idiom {
        name: "pm.response.to.be.json on html must fail",
        phase: Phase::NotFound,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.json; });"#,
    },
    Idiom {
        name: "pm.response.to.be.success",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.success; });"#,
    },
    Idiom {
        name: "pm.response.to.be.error on a 404",
        phase: Phase::NotFound,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.error; });"#,
    },
    Idiom {
        name: "pm.response.to.be.clientError",
        phase: Phase::NotFound,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.clientError; });"#,
    },
    Idiom {
        name: "pm.response.to.be.notFound",
        phase: Phase::NotFound,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.notFound; });"#,
    },
    Idiom {
        name: "pm.response.to.be.unauthorized on a 404 must fail",
        phase: Phase::NotFound,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.unauthorized; });"#,
    },
    Idiom {
        name: "pm.response.to.be.withBody",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.withBody; });"#,
    },
    Idiom {
        name: "unknown pm.response.to.be.<x> is rejected",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.response.to.be.teapot;"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonBody()",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.jsonBody(); });"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonBody() on html must fail",
        phase: Phase::NotFound,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.have.jsonBody(); });"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonBody(path, object)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.jsonBody("user", { name: "nova", email: "nova@x.dev" }); });"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonBody(wholeBodyMismatch) must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.have.jsonBody({ id: 7 }); });"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonBody(path, value)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.jsonBody("user.name", "nova"); });"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonBody(path, wrong) must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.have.jsonBody("user.name", "zed"); });"#,
    },
    Idiom {
        name: "pm.response.to.have.header(name)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.header("Content-Type"); });"#,
    },
    Idiom {
        name: "pm.response.to.have.header(name, value)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.header("Content-Type", "application/json"); });"#,
    },
    Idiom {
        name: "pm.response.to.have.header(missing) must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.have.header("X-Nope"); });"#,
    },
    Idiom {
        name: "pm.response.to.have.body(text)",
        phase: Phase::NotFound,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.have.body("<html>nope</html>"); });"#,
    },
    Idiom {
        name: "pm.response.to.have.jsonSchema(...)",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.response.to.have.jsonSchema({ type: "object" });"#,
    },
    Idiom {
        name: "const json = pm.response.json(); expect(json.data.length).to.eql(3)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"const json = pm.response.json(); pm.test("t", function () { pm.expect(json.data.length).to.eql(3); });"#,
    },
    Idiom {
        name: "pm.expect(json).to.have.property(\"id\")",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json()).to.have.property("id"); });"#,
    },
    Idiom {
        name: "pm.expect(json).to.have.property(\"id\", 7)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json()).to.have.property("id", 7); });"#,
    },
    Idiom {
        name: "pm.expect(res).to.be.an(\"object\")",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json()).to.be.an("object"); });"#,
    },
    Idiom {
        name: "pm.expect(arr).to.be.an(\"array\").with.lengthOf(3)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().data).to.be.an("array").with.lengthOf(3); });"#,
    },
    Idiom {
        name: "pm.expect(obj).to.deep.equal(obj)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().user).to.deep.equal({ name: "nova", email: "nova@x.dev" }); });"#,
    },
    Idiom {
        name: "pm.expect(arr).to.have.members([...])",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect([1, 2, 3]).to.have.members([3, 1, 2]); });"#,
    },
    Idiom {
        name: "pm.expect(x).to.be.oneOf([...])",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().id).to.be.oneOf([1, 7, 9]); });"#,
    },
    Idiom {
        name: "pm.expect(x).that.is.a(\"string\")",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().token).that.is.a("string"); });"#,
    },
    Idiom {
        name: ".and chaining after an assertion",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().token).to.be.a("string").and.to.have.lengthOf(3); });"#,
    },
    Idiom {
        name: "pm.expect(obj).to.have.all.keys(...)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().user).to.have.all.keys("name", "email"); });"#,
    },
    Idiom {
        name: "pm.expect(undefined).to.exist must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().nope).to.exist; });"#,
    },
    Idiom {
        name: "pm.expect(x).to.be.within(a, b)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().count).to.be.within(1, 5); });"#,
    },
    Idiom {
        name: "pm.expect(str).to.have.string(...)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.text()).to.have.string("nova"); });"#,
    },
    Idiom {
        name: "pm.expect(x).to.match(/re/)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.json().user.email).to.match(/@x\.dev$/); });"#,
    },
    Idiom {
        name: "pm.expect.fail(message)",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.expect.fail("explicit failure"); });"#,
    },
    Idiom {
        name: "unknown chai assertion is rejected",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.expect(1).to.be.bananas;"#,
    },
    Idiom {
        name: "pm.expect(pm.response.responseTime).to.be.below(500)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", () => { pm.expect(pm.response.responseTime).to.be.below(500); });"#,
    },
    Idiom {
        name: "pm.response.headers.get is case-insensitive",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.headers.get("content-type")).to.include("json"); });"#,
    },
    Idiom {
        name: "pm.response.json() on a non-JSON body",
        phase: Phase::NotFound,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.json(); });"#,
    },
    Idiom {
        name: "pm.environment.set from the response",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.environment.set("token", pm.response.json().token); pm.test("t", function () { pm.expect(pm.environment.get("token")).to.eql("t0k"); });"#,
    },
    Idiom {
        name: "pm.collectionVariables.set / pm.variables.get",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.collectionVariables.set("userId", 7); pm.test("t", function () { pm.expect(pm.variables.get("userId")).to.eql("7"); });"#,
    },
    Idiom {
        name: "pm.globals.set / pm.globals.get",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.globals.set("g", "1"); pm.test("t", function () { pm.expect(pm.globals.get("g")).to.eql("1"); });"#,
    },
    Idiom {
        name: "pm.variables.replaceIn(\"{{base}}/x\")",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.variables.replaceIn("{{base}}/x")).to.eql("https://api.x.dev/x"); });"#,
    },
    Idiom {
        name: "pm.variables.replaceIn(\"{{$guid}}\")",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.variables.replaceIn("{{$guid}}")).to.match(/^[0-9a-f-]{36}$/); });"#,
    },
    Idiom {
        name: "pm.request.headers.add({key, value})",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.request.headers.add({ key: "X-Nonce", value: String(Date.now()) }); pm.test("t", function () { pm.expect(pm.request.headers.has("X-Nonce")).to.be.true; });"#,
    },
    Idiom {
        name: "pm.request.url assignment",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.request.url = pm.request.url + "?trace=1"; pm.test("t", function () { pm.expect(String(pm.request.url)).to.include("trace=1"); });"#,
    },
    Idiom {
        name: "pm.request.url.toString()",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.request.url.toString()).to.include("/users/7"); });"#,
    },
    Idiom {
        name: "pm.request.url.query.add({key, value})",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.request.url.query.add({ key: "page", value: "2" }); pm.test("t", function () { pm.expect(pm.request.url.toString()).to.include("page=2"); });"#,
    },
    Idiom {
        name: "Date.now() and Math.random()",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(Date.now()).to.be.above(0); pm.expect(Math.random()).to.be.below(1); });"#,
    },
    Idiom {
        name: "pm.info.requestName",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.info.requestName).to.eql("Get user"); });"#,
    },
    Idiom {
        name: "pm.info.requestId",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.info.requestId).to.be.a("string"); });"#,
    },
    Idiom {
        name: "pm.info.iterationCount",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.info.iterationCount).to.eql(1); });"#,
    },
    Idiom {
        name: "postman.setEnvironmentVariable (legacy)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"postman.setEnvironmentVariable("token", "t0k"); pm.test("t", function () { pm.expect(postman.getEnvironmentVariable("token")).to.eql("t0k"); });"#,
    },
    Idiom {
        name: "postman.setGlobalVariable (legacy)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"postman.setGlobalVariable("g", "1"); pm.test("t", function () { pm.expect(pm.globals.get("g")).to.eql("1"); });"#,
    },
    Idiom {
        name: "tests[\"...\"] = ... (legacy sandbox)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"tests["Status code is 200"] = responseCode.code === 200;"#,
    },
    Idiom {
        name: "responseBody / responseTime (legacy sandbox)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(responseBody).to.include("nova"); pm.expect(responseTime).to.be.below(500); });"#,
    },
    Idiom {
        name: "postman.setNextRequest (legacy flow control)",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"postman.setNextRequest("Login");"#,
    },
    Idiom {
        name: "pm.execution.setNextRequest",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.execution.setNextRequest("Login");"#,
    },
    Idiom {
        name: "pm.sendRequest",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"pm.sendRequest("https://x.dev/token", function (err, res) {});"#,
    },
    Idiom {
        name: "pm.iterationData.get",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"var u = pm.iterationData.get("userId");"#,
    },
    Idiom {
        name: "pm.cookies.get",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.cookies.get("session");"#,
    },
    Idiom {
        name: "require('crypto-js')",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"var CryptoJS = require("crypto-js");"#,
    },
    Idiom {
        name: "CryptoJS global",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"var sig = CryptoJS.HmacSHA256("a", "b").toString();"#,
    },
    Idiom {
        name: "lodash _ global",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"var ids = _.map(pm.response.json().data, "id");"#,
    },
    Idiom {
        name: "moment global",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"var now = moment().format();"#,
    },
    Idiom {
        name: "xml2Json global",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"var doc = xml2Json(pm.response.text());"#,
    },
    Idiom {
        name: "btoa / atob",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(atob(btoa("user:pass"))).to.eql("user:pass"); });"#,
    },
    Idiom {
        name: "pm.test with a done callback",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.test("t", function (done) { done(); });"#,
    },
    Idiom {
        name: "modern JS: let/const, arrows, template literals, spread",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"const { data } = pm.response.json(); const ids = data.map(d => d.id); const all = [...ids, 4]; pm.test(`ids ${ids.length}`, () => { pm.expect(all).to.have.lengthOf(4); });"#,
    },
    Idiom {
        name: "pm.response.to.be.error on a 200 must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.error; });"#,
    },
    Idiom {
        name: "pm.response.to.be.notFound on a 200 must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.notFound; });"#,
    },
    Idiom {
        name: "pm.response.to.be.success on a 404 must fail",
        phase: Phase::NotFound,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.success; });"#,
    },
    Idiom {
        name: "pm.response.to.be.clientError on a 200 must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.clientError; });"#,
    },
    Idiom {
        name: "pm.response.to.be.withBody on a 204 must fail",
        phase: Phase::Empty,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.withBody; });"#,
    },
    Idiom {
        name: "pm.response.to.be.html",
        phase: Phase::NotFound,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.be.html; });"#,
    },
    Idiom {
        name: "pm.response.to.be.html on JSON must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.html; });"#,
    },
    Idiom {
        name: "pm.response.to.be.accepted on a 200 must fail",
        phase: Phase::Post,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.be.accepted; });"#,
    },
    Idiom {
        name: "pm.response.to.have.body() on an empty body must fail",
        phase: Phase::Empty,
        expect: Expect::Fail,
        source: r#"pm.test("t", function () { pm.response.to.have.body(); });"#,
    },
    Idiom {
        name: "pm.response.to.not.have.status(...)",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.response.to.not.have.status(500); });"#,
    },
    Idiom {
        name: "pm.response.reason()",
        phase: Phase::NotFound,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.reason()).to.eql("Not Found"); });"#,
    },
    Idiom {
        name: "pm.response.size().body",
        phase: Phase::Post,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.response.size().body).to.be.above(0); });"#,
    },
    Idiom {
        name: "pm.request.url.query.remove / .toObject",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.request.url = "https://x.dev/a?keep=1&drop=2"; pm.request.url.query.remove("drop"); pm.test("t", function () { pm.expect(pm.request.url.query.toObject()).to.eql({ keep: "1" }); });"#,
    },
    Idiom {
        name: "pm.request.url.replace (string method on a Url object)",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"var u = pm.request.url.replace("http", "https");"#,
    },
    Idiom {
        name: "pm.request.body.raw",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.request.body = '{"a":1}'; pm.test("t", function () { pm.expect(JSON.parse(pm.request.body.raw).a).to.eql(1); });"#,
    },
    Idiom {
        name: "pm.variables.replaceIn(\"{{$timestamp}}\")",
        phase: Phase::Pre,
        expect: Expect::Pass,
        source: r#"pm.test("t", function () { pm.expect(pm.variables.replaceIn("{{$timestamp}}")).to.match(/^[0-9]{10}$/); });"#,
    },
    Idiom {
        name: "pm.variables.replaceIn with an unknown dynamic variable",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"pm.variables.replaceIn("{{$randomBankAccountName}}");"#,
    },
    Idiom {
        name: "pm.vault.get",
        phase: Phase::Pre,
        expect: Expect::Loud,
        source: r#"pm.vault.get("key");"#,
    },
    Idiom {
        name: "pm.visualizer.set",
        phase: Phase::Post,
        expect: Expect::Loud,
        source: r#"pm.visualizer.set("<b>{{x}}</b>", {});"#,
    },
];

#[test]
fn postman_idiom_compatibility_matrix() {
    let mut rows = Vec::new();
    let mut confusing = Vec::new();
    for idiom in IDIOMS {
        let (status, detail) = classify(idiom);
        if status == Status::Confusing {
            confusing.push(format!("{} — {detail}", idiom.name));
        }
        rows.push(format!(
            "| `{}` | {} | {} |",
            idiom.name.replace('|', "\\|"),
            status.label(),
            detail.replace('|', "\\|").replace('\n', " ")
        ));
    }
    println!("\n| idiom | status | detail |");
    println!("| --- | --- | --- |");
    for row in &rows {
        println!("{row}");
    }
    println!(
        "\n{} idioms measured, {} confusing\n",
        IDIOMS.len(),
        confusing.len()
    );
    assert!(
        confusing.is_empty(),
        "idioms that fail confusingly or silently:\n  {}",
        confusing.join("\n  ")
    );
}

#[test]
fn expect_fail_reports_the_message_it_was_given() {
    let out = script::run_script(
        r#"pm.test("t", function () { pm.expect.fail("explicit failure"); });"#,
        context(Phase::Post),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(out.tests[0].error.as_deref(), Some("explicit failure"));
}

#[test]
fn a_falsy_legacy_test_is_recorded_as_a_failure() {
    let out = script::run_script(
        r#"tests["Status code is 201"] = responseCode.code === 201;"#,
        context(Phase::Post),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(out.tests.len(), 1);
    assert!(!out.tests[0].passed);
    assert!(out.tests[0].error.as_ref().unwrap().contains("falsy"));
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman")
            .join(name),
    )
    .unwrap()
}

fn all_requests(workspace: &std::path::Path) -> Vec<(String, SavedRequest)> {
    fn walk(
        workspace: &std::path::Path,
        slug: &str,
        folders: &[collection::FolderNode],
        summaries: &[collection::RequestSummary],
        out: &mut Vec<(String, SavedRequest)>,
    ) {
        for summary in summaries {
            out.push((
                summary.path.clone(),
                collection::load_request(workspace, slug, &summary.path).unwrap(),
            ));
        }
        for folder in folders {
            walk(workspace, slug, &folder.folders, &folder.requests, out);
        }
    }
    let mut out = Vec::new();
    for node in collection::list_tree(workspace).unwrap().collections {
        walk(
            workspace,
            &node.slug,
            &node.folders,
            &node.requests,
            &mut out,
        );
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn seeded_context(request: &SavedRequest, phase: Phase) -> ScriptContext {
    let mut ctx = context(phase);
    ctx.request_name = request.name.clone();
    ctx.request = ScriptRequest {
        method: request.method.clone(),
        url: request.url.clone(),
        headers: request.headers.clone(),
        body: request.body.as_text().map(String::from),
    };
    ctx.vars
        .insert("accessToken".to_string(), "seeded".to_string());
    ctx.vars
        .insert("baseUrl".to_string(), "https://api.acme.test".to_string());
    ctx
}

fn run_every_script(fixture_name: &str, expected_loud: &[&str]) -> (usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    postman::import(dir.path(), &fixture(fixture_name)).unwrap();
    let mut passed = 0;
    let mut ran = 0;
    let mut confusing = Vec::new();
    println!("\n### {fixture_name}\n");
    println!("| request | phase | result |");
    println!("| --- | --- | --- |");
    for (_, request) in all_requests(dir.path()) {
        for (phase, source) in [
            (Phase::Pre, request.scripts.pre.clone()),
            (Phase::Post, request.scripts.post.clone()),
        ] {
            let Some(source) = source.filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            ran += 1;
            let label = if phase == Phase::Pre { "pre" } else { "test" };
            match script::run_script(&source, seeded_context(&request, phase), Limits::default()) {
                Ok(out) => {
                    let failed: Vec<&str> = out
                        .tests
                        .iter()
                        .filter(|t| !t.passed)
                        .map(|t| t.name.as_str())
                        .collect();
                    if failed.is_empty() {
                        passed += 1;
                    } else {
                        confusing.push(format!(
                            "{} ({label}): {} failed",
                            request.name,
                            failed.join(", ")
                        ));
                    }
                    println!(
                        "| {} | {label} | {} assertions, {} failed |",
                        request.name,
                        out.tests.len(),
                        failed.len()
                    );
                }
                Err(e) => {
                    let message = e.to_string();
                    println!(
                        "| {} | {label} | stopped: {} |",
                        request.name,
                        message.lines().next().unwrap_or_default()
                    );
                    if !expected_loud.contains(&request.name.as_str())
                        || !named_limitation(&message)
                    {
                        confusing.push(format!("{} ({label}): {message}", request.name));
                    }
                }
            }
        }
    }
    assert!(
        confusing.is_empty(),
        "scripts that did not behave as advertised:\n  {}",
        confusing.join("\n  ")
    );
    (ran, passed)
}

#[test]
fn a_realistic_crud_collection_runs_every_script_it_ships_with() {
    let (ran, passed) = run_every_script("rest_crud.json", &[]);
    assert_eq!(ran, 14);
    assert_eq!(passed, 14);
}

#[test]
fn the_edge_case_collection_only_stops_on_documented_limitations() {
    let (ran, passed) = run_every_script("edge_cases.json", &["Unsupported script apis"]);
    assert_eq!(ran, 6);
    assert_eq!(passed, 4);
}

#[test]
fn the_crud_collection_imports_with_the_expected_report() {
    let dir = tempfile::tempdir().unwrap();
    let report = postman::import(dir.path(), &fixture("rest_crud.json")).unwrap();
    assert_eq!(report.imported, 7);
    assert_eq!(report.collections, 1);
    assert_eq!(report.environments, 1);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert_eq!(
        report.warnings,
        vec![
            "Login: the description was dropped — neither .http nor .grpc has a line for one; keep it in a `#` comment above the request",
            "Refresh: disabled form fields (scope) were dropped — a .http file writes the whole form body as one line of text",
            "Users: pre-request script copied into every request below it (List users, Create user, Get user, Delete user) — Mándalo has no shared scripts, so edit each copy",
            "Acme API: pre-request and test scripts copied into every request below it (Login, Refresh, List users, Create user, Get user and 2 more) — Mándalo has no shared scripts, so edit each copy"
        ]
    );

    let requests = all_requests(dir.path());
    let login = requests.iter().find(|(_, r)| r.name == "Login").unwrap();
    assert_eq!(
        login.1.auth,
        mandalo_core::request::Auth::None,
        "the request opts out with noauth"
    );
    assert!(login
        .1
        .scripts
        .pre
        .as_ref()
        .unwrap()
        .contains("X-Correlation-Id"));
    assert!(login
        .1
        .scripts
        .post
        .as_ref()
        .unwrap()
        .contains("Response time is acceptable"));

    let get_user = requests.iter().find(|(_, r)| r.name == "Get user").unwrap();
    assert_eq!(get_user.1.url, "{{baseUrl}}/users/{{userId}}");

    let health = requests.iter().find(|(_, r)| r.name == "Health").unwrap();
    assert_eq!(health.1.url, "{{baseUrl}}/health");

    let list = requests
        .iter()
        .find(|(_, r)| r.name == "List users")
        .unwrap();
    assert_eq!(list.1.url, "{{baseUrl}}/users?page=1&per_page=25");
    assert_eq!(
        list.1.auth,
        mandalo_core::request::Auth::Bearer {
            token: "{{accessToken}}".to_string()
        }
    );
    assert_eq!(
        list.1.headers,
        vec![("Accept".to_string(), "application/json".to_string())]
    );
}

#[test]
fn the_edge_case_collection_reports_every_loss_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let report = postman::import(dir.path(), &fixture("edge_cases.json")).unwrap();
    assert_eq!(report.imported, 15);
    assert_eq!(
        report.skipped,
        vec![
            "Upload avatar: 2 form-data fields not imported — a .http file cannot express a multipart body, so the request was imported without one"
        ]
    );
    for expected in [
        "OAuth2 stored token: OAuth 2.0 imported as the access token Postman had stored",
        "AWS v4: awsv4 auth is not supported",
        "Digest: digest auth is not supported",
        "Hawk: hawk auth is not supported",
        "NTLM: ntlm auth is not supported",
        "Unsupported script apis: script uses pm.sendRequest",
        "Unsupported script apis: script uses setNextRequest",
        "Unsupported script apis: script uses jsonSchema",
        "Upload avatar: file field `avatar` needs a file inside the workspace — the export referenced /Users/dev/avatar.png",
        "Raw file body: the binary body needs a file inside the workspace — the export referenced /Users/dev/blob.bin",
    ] {
        assert!(
            report.warnings.iter().any(|w| w.starts_with(expected)),
            "missing warning {expected:?} in {:#?}",
            report.warnings
        );
    }
    let requests = all_requests(dir.path());
    let upload = requests
        .iter()
        .find(|(_, r)| r.name == "Upload avatar")
        .unwrap();
    assert_eq!(
        upload.1.body,
        mandalo_core::Body::None,
        "a .http file cannot carry a multipart body, so the fields are reported instead"
    );
    let oauth = requests
        .iter()
        .find(|(_, r)| r.name == "OAuth2 stored token")
        .unwrap();
    assert_eq!(
        oauth.1.auth,
        mandalo_core::request::Auth::Bearer {
            token: "{{oauthToken}}".to_string()
        }
    );
}

#[test]
fn an_exported_environment_imports_without_its_disabled_values() {
    let dir = tempfile::tempdir().unwrap();
    let report = postman::import(dir.path(), &fixture("staging_environment.json")).unwrap();
    assert_eq!(report.environments, 1);
    assert_eq!(report.imported, 0);
    let envs = mandalo_core::workspace::list_environments(dir.path())
        .unwrap()
        .items;
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].name, "Acme--staging-");
    assert_eq!(envs[0].vars.len(), 4);
    assert!(!envs[0].vars.contains_key("retired"));
    assert_eq!(
        envs[0].vars.get("baseUrl").unwrap(),
        "https://staging.acme.test"
    );
}
