use crate::assertions::{evaluate_tests, resolve_capture, CaptureScope, TestResult};
use crate::capability::{Decision, HostPolicy, SecretStore};
use crate::collection::{self, SavedRequest};
use crate::error::{CoreError, CoreResult};
use crate::grpc::{self, GrpcResponse, GrpcSpec};
use crate::interpolate;
use crate::request::{self, Auth, RequestSpec, ResponseData};
use crate::script::{self, Limits, ScriptContext, ScriptRequest, ScriptResponse, ScriptTest};
use crate::workspace::{self, EnvDoc, Environment, VarDef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarFrame {
    pub env: String,
    pub vars: BTreeMap<String, String>,
    /// Variables the environment declares secret, mapped to the hosts they may
    /// be sent to. An empty list means "declared but not bound to a host yet".
    pub secrets: BTreeMap<String, Vec<String>>,
}

impl VarFrame {
    pub fn new(env: impl Into<String>) -> Self {
        VarFrame {
            env: env.into(),
            vars: BTreeMap::new(),
            secrets: BTreeMap::new(),
        }
    }

    pub fn from_environment(env: &Environment) -> Self {
        VarFrame {
            env: env.name.clone(),
            vars: env.vars.clone(),
            secrets: BTreeMap::new(),
        }
    }

    pub fn from_doc(doc: &EnvDoc) -> Self {
        let mut frame = VarFrame::new(doc.name.clone());
        for (key, def) in &doc.vars {
            match def {
                VarDef::Plain { value } => {
                    frame.vars.insert(key.clone(), value.clone());
                }
                VarDef::Secret { hosts } => {
                    frame.secrets.insert(key.clone(), hosts.clone());
                }
            }
        }
        frame
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) {
        self.vars.remove(key);
    }
}

/// One capture that actually ran, carrying the value it resolved to. `captured` on
/// [`StepResult`] keeps the same data as a flat name→value map for the desktop app.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutcome {
    pub from: String,
    pub into: String,
    pub value: String,
    pub scope: CaptureScope,
}

/// A secret this request used that is not bound to any host yet, with the host it
/// went to — enough for a UI to offer "bind {name} to {host}".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnboundSecret {
    pub name: String,
    pub env: String,
    pub host: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub request_name: String,
    pub path: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub url: String,
    pub response: Option<ResponseData>,
    pub grpc: Option<GrpcResponse>,
    pub tests: Vec<TestResult>,
    pub script_tests: Vec<ScriptTest>,
    pub logs: Vec<String>,
    pub captured: BTreeMap<String, String>,
    #[serde(default)]
    pub captures: Vec<CaptureOutcome>,
    #[serde(default)]
    pub unbound_secrets: Vec<UnboundSecret>,
    /// Every plain variable the scripts wrote, pre and post merged in order.
    #[serde(default)]
    pub var_sets: BTreeMap<String, String>,
    #[serde(default)]
    pub var_unsets: Vec<String>,
    /// Names only: a write to a declared secret is reported so a caller can say
    /// "prod.token was updated" without the value ever leaving the runner.
    #[serde(default)]
    pub secret_var_sets: Vec<String>,
    pub passed: bool,
    pub duration_ms: u128,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

impl StepResult {
    fn empty(request_name: &str, path: &str) -> Self {
        StepResult {
            request_name: request_name.to_string(),
            path: path.to_string(),
            method: String::new(),
            url: String::new(),
            response: None,
            grpc: None,
            tests: Vec::new(),
            script_tests: Vec::new(),
            logs: Vec::new(),
            captured: BTreeMap::new(),
            captures: Vec::new(),
            unbound_secrets: Vec::new(),
            var_sets: BTreeMap::new(),
            var_unsets: Vec::new(),
            secret_var_sets: Vec::new(),
            passed: false,
            duration_ms: 0,
            error: None,
            error_code: None,
        }
    }

    pub fn failed(request_name: &str, path: &str, error: &CoreError, duration_ms: u128) -> Self {
        let mut step = StepResult::empty(request_name, path);
        step.error = Some(error.to_string());
        step.error_code = Some(error.code().to_string());
        step.duration_ms = duration_ms;
        step
    }

    pub fn failures(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(error) = &self.error {
            out.push(error.clone());
        }
        for test in &self.tests {
            if !test.passed {
                out.push(match &test.detail {
                    Some(detail) => format!("{}: {detail}", test.name),
                    None => test.name.clone(),
                });
            }
        }
        for test in &self.script_tests {
            if !test.passed {
                out.push(match &test.error {
                    Some(detail) => format!("{}: {detail}", test.name),
                    None => test.name.clone(),
                });
            }
        }
        out
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub collection: String,
    pub environment: Option<String>,
    pub steps: Vec<StepResult>,
    pub total: usize,
    pub failed: usize,
    pub passed: bool,
    pub duration_ms: u128,
}

pub struct Runner<S: SecretStore, P: HostPolicy> {
    secrets: S,
    policy: P,
    limits: Limits,
    workspace: Option<std::path::PathBuf>,
}

impl<S: SecretStore, P: HostPolicy> Runner<S, P> {
    pub fn new(secrets: S, policy: P) -> Self {
        Runner {
            secrets,
            policy,
            limits: Limits::default(),
            workspace: None,
        }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Root that `formdata` and `binary` body files resolve against. Without it
    /// a request that carries a file fails loudly instead of reading anything.
    pub fn with_workspace(mut self, workspace: impl Into<std::path::PathBuf>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    pub async fn run_request(
        &self,
        req: &SavedRequest,
        vars: &mut VarFrame,
    ) -> CoreResult<StepResult> {
        self.run_request_in(None, req, vars).await
    }

    /// `workspace` overrides the runner's own root for this request — a suite
    /// run already knows which workspace it is reading from.
    pub async fn run_request_in(
        &self,
        workspace: Option<&Path>,
        req: &SavedRequest,
        vars: &mut VarFrame,
    ) -> CoreResult<StepResult> {
        let started = Instant::now();
        let mut step = StepResult::empty(&req.name, "");
        let mut writes = VarWrites::default();
        step.method = req.method.clone();
        step.url = req.url.clone();
        let mut wire = ScriptRequest {
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req.headers.clone(),
            body: req.body.as_text().map(String::from),
        };

        if let Some(source) = req.scripts.pre.as_deref().filter(|s| !s.trim().is_empty()) {
            let outcome = script::run_script(
                source,
                ScriptContext {
                    vars: vars.vars.clone(),
                    request_name: req.name.clone(),
                    request: wire.clone(),
                    response: None,
                },
                self.limits,
            )?;
            apply_var_writes(vars, &outcome.var_sets, &outcome.var_unsets);
            writes.record(&outcome.var_sets, &outcome.var_unsets);
            step.script_tests.extend(outcome.tests);
            step.logs.extend(outcome.logs);
            if let Some(patch) = outcome.request_patch {
                wire = patch;
            }
            step.method = wire.method.clone();
            step.url = wire.url.clone();
        }

        let used_secrets = self.resolve_secrets(req, &wire, vars)?;

        let (response, grpc_response) = if req.kind == "grpc" {
            let spec = self.grpc_spec(req, &wire, vars)?;
            let url = interpolate::apply(&spec.url, &vars.vars)?;
            step.unbound_secrets = enforce_secret_hosts(&used_secrets, vars, &url)?;
            self.check_host(&url).await?;
            (None, Some(grpc::send_grpc(spec).await?))
        } else {
            let spec = RequestSpec {
                kind: req.kind.clone(),
                method: wire.method.clone(),
                url: wire.url.clone(),
                headers: wire.headers.clone(),
                body: req.body.with_text(wire.body.clone())?,
                auth: req.auth.clone(),
                vars: vars.vars.clone(),
                workspace: workspace
                    .map(Path::to_path_buf)
                    .or_else(|| self.workspace.clone()),
            };
            let client = request::client()?;
            let built = request::build(client, &spec)?;
            step.unbound_secrets = enforce_secret_hosts(&used_secrets, vars, built.url().as_str())?;
            self.check_host(built.url().as_str()).await?;
            (Some(request::send_built(client, built).await?), None)
        };

        let (status, headers, body, duration_ms) = match (&response, &grpc_response) {
            (Some(r), _) => (r.status, r.headers.clone(), r.body.clone(), r.duration_ms),
            (_, Some(g)) => (0, Vec::new(), g.body.clone(), g.duration_ms),
            _ => (0, Vec::new(), String::new(), 0),
        };

        if let Some(source) = req.scripts.post.as_deref().filter(|s| !s.trim().is_empty()) {
            let outcome = script::run_script(
                source,
                ScriptContext {
                    vars: vars.vars.clone(),
                    request_name: req.name.clone(),
                    request: wire.clone(),
                    response: Some(ScriptResponse {
                        status,
                        status_text: response
                            .as_ref()
                            .map(|r| r.status_text.clone())
                            .unwrap_or_default(),
                        headers: headers.clone(),
                        body: body.clone(),
                        duration_ms,
                    }),
                },
                self.limits,
            )?;
            apply_var_writes(vars, &outcome.var_sets, &outcome.var_unsets);
            writes.record(&outcome.var_sets, &outcome.var_unsets);
            step.script_tests.extend(outcome.tests);
            step.logs.extend(outcome.logs);
        }

        let (plain, unsets, secret) = writes.split_secrets(vars);
        step.var_sets = plain;
        step.var_unsets = unsets;
        step.secret_var_sets = secret;

        step.tests = evaluate_tests(&req.tests, status, &headers, &body, duration_ms);

        for capture in &req.captures {
            let value =
                resolve_capture(&capture.from, status, &headers, &body)?.ok_or_else(|| {
                    CoreError::Capture(format!("capture {:?} matched nothing", capture.from))
                })?;
            vars.set(capture.into.clone(), value.clone());
            step.captured.insert(capture.into.clone(), value.clone());
            step.captures.push(CaptureOutcome {
                from: capture.from.clone(),
                into: capture.into.clone(),
                value,
                scope: capture.scope,
            });
        }

        step.response = response;
        step.grpc = grpc_response;
        step.passed = step.tests.iter().all(|t| t.passed)
            && step.script_tests.iter().all(|t| t.passed)
            && step.error.is_none();
        step.duration_ms = started.elapsed().as_millis();
        Ok(step)
    }

    /// One request against a workspace environment, whether it lives on disk or
    /// is still an unsaved draft in an editor. Same pipeline as a suite step.
    pub async fn run_one(
        &self,
        ws: &Path,
        req: &SavedRequest,
        env: Option<&str>,
    ) -> CoreResult<StepResult> {
        let mut vars = env_frame(ws, env)?;
        self.run_request_in(Some(ws), req, &mut vars).await
    }

    pub async fn run_suite(
        &self,
        ws: &Path,
        collection_slug: &str,
        filter: Option<&str>,
        env: Option<&str>,
    ) -> CoreResult<RunReport> {
        self.run_suite_with(ws, collection_slug, filter, env, false)
            .await
    }

    pub async fn run_suite_with(
        &self,
        ws: &Path,
        collection_slug: &str,
        filter: Option<&str>,
        env: Option<&str>,
        fail_fast: bool,
    ) -> CoreResult<RunReport> {
        let started = Instant::now();
        let mut vars = env_frame(ws, env)?;

        let mut report = RunReport {
            collection: collection_slug.to_string(),
            environment: env.map(String::from),
            steps: Vec::new(),
            total: 0,
            failed: 0,
            passed: true,
            duration_ms: 0,
        };

        for path in suite_paths(ws, collection_slug, filter)? {
            let request = collection::load_request(ws, collection_slug, &path)?;
            let step_started = Instant::now();
            let mut step = match self.run_request_in(Some(ws), &request, &mut vars).await {
                Ok(step) => step,
                Err(e) => {
                    let mut step = StepResult::failed(
                        &request.name,
                        &path,
                        &e,
                        step_started.elapsed().as_millis(),
                    );
                    step.method = request.method.clone();
                    step.url = request.url.clone();
                    step
                }
            };
            step.path = path;
            report.total += 1;
            if !step.passed {
                report.failed += 1;
                report.passed = false;
            }
            let stop = fail_fast && !step.passed;
            report.steps.push(step);
            if stop {
                break;
            }
        }

        report.duration_ms = started.elapsed().as_millis();
        Ok(report)
    }

    fn grpc_spec(
        &self,
        req: &SavedRequest,
        wire: &ScriptRequest,
        vars: &VarFrame,
    ) -> CoreResult<GrpcSpec> {
        let g = req
            .grpc
            .as_ref()
            .ok_or_else(|| CoreError::Request("grpc request is missing the grpc body".into()))?;
        Ok(GrpcSpec {
            url: wire.url.clone(),
            proto_paths: g.proto_paths.clone(),
            service: g.service.clone(),
            method: g.method.clone(),
            message: g.message.clone(),
            metadata: g.metadata.clone(),
            vars: vars.vars.clone(),
        })
    }

    /// Resolves every `{{name}}` the request references that the environment does
    /// not already hold, and returns the declared-secret names it used. A declared
    /// secret with no value on this machine is a hard stop: an empty header is a
    /// silent 401 at best and a leak of the request body to an unauthenticated
    /// endpoint at worst.
    fn resolve_secrets(
        &self,
        req: &SavedRequest,
        wire: &ScriptRequest,
        vars: &mut VarFrame,
    ) -> CoreResult<Vec<String>> {
        let mut templates: Vec<&str> = vec![&wire.method, &wire.url];
        for (k, v) in &wire.headers {
            templates.push(k);
            templates.push(v);
        }
        if let Some(body) = &wire.body {
            templates.push(body);
        }
        templates.extend(req.body.templates());
        if let Some(g) = &req.grpc {
            templates.push(&g.message);
            for (k, v) in &g.metadata {
                templates.push(k);
                templates.push(v);
            }
        }
        match &req.auth {
            Auth::None => {}
            Auth::Bearer { token } => templates.push(token),
            Auth::Basic { username, password } => {
                templates.push(username);
                templates.push(password);
            }
            Auth::Apikey { key, value, .. } => {
                templates.push(key);
                templates.push(value);
            }
        }

        let mut used: Vec<String> = Vec::new();
        for template in templates {
            for name in interpolate::names(template) {
                if vars.secrets.contains_key(&name) {
                    // A secret stays "used" even when an earlier step already
                    // resolved it, or the host binding would only cover the
                    // first request of a suite.
                    if !used.contains(&name) {
                        used.push(name.clone());
                    }
                    if vars.vars.contains_key(&name) {
                        continue;
                    }
                    let value = self.secrets.get(&vars.env, &name)?.ok_or_else(|| {
                        CoreError::Secret(format!(
                            "{}.{name} is declared secret but has no value on this machine — store it with set_secret, or export {}",
                            vars.env,
                            crate::capability::EnvVarStore::variable_name(&vars.env, &name)
                        ))
                    })?;
                    crate::redact::register(&vars.env, &name, &value);
                    vars.set(name, value);
                    continue;
                }
                if vars.vars.contains_key(&name) {
                    continue;
                }
                if let Some(value) = self.secrets.get(&vars.env, &name)? {
                    crate::redact::register(&vars.env, &name, &value);
                    vars.set(name, value);
                }
            }
        }
        Ok(used)
    }

    async fn check_host(&self, url: &str) -> CoreResult<()> {
        if !self.policy.enforces() {
            return Ok(());
        }
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| CoreError::Request(format!("invalid url {url:?}: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| CoreError::Request(format!("url has no host: {url}")))?
            .to_string();
        let port = parsed.port_or_known_default().unwrap_or(443);

        let addresses: Vec<IpAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|e| CoreError::Network(format!("cannot resolve {host}: {e}")))?
                .map(|addr| addr.ip())
                .collect(),
        };
        if addresses.is_empty() {
            return Err(CoreError::Network(format!("{host} resolved to no address")));
        }
        for ip in addresses {
            match self.policy.allow(&host, &ip) {
                Decision::Allow => {}
                Decision::Deny(reason) => {
                    return Err(CoreError::HostDenied(format!(
                        "{host} is blocked: {reason}"
                    )))
                }
                Decision::Ask => {
                    return Err(CoreError::HostConfirmRequired(format!(
                        "{host} needs confirmation before it can be called"
                    )))
                }
            }
        }
        Ok(())
    }
}

fn host_of(url: &str) -> CoreResult<String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| CoreError::Request(format!("invalid url {url:?}: {e}")))?;
    Ok(parsed
        .host_str()
        .ok_or_else(|| CoreError::Request(format!("url has no host: {url}")))?
        .to_ascii_lowercase())
}

/// A secret bound to hosts may only travel to those hosts. An unbound secret is
/// allowed through and reported, so a UI can offer to bind it to the host it saw.
fn enforce_secret_hosts(
    used: &[String],
    vars: &VarFrame,
    url: &str,
) -> CoreResult<Vec<UnboundSecret>> {
    if used.is_empty() {
        return Ok(Vec::new());
    }
    let host = host_of(url)?;
    let mut unbound = Vec::new();
    for name in used {
        let Some(hosts) = vars.secrets.get(name) else {
            continue;
        };
        if hosts.is_empty() {
            unbound.push(UnboundSecret {
                name: name.clone(),
                env: vars.env.clone(),
                host: host.clone(),
            });
            continue;
        }
        if !hosts.iter().any(|h| h == &host) {
            return Err(CoreError::SecretHostDenied(format!(
                "{}.{name} is bound to {} and must not be sent to {host}",
                vars.env,
                hosts.join(", ")
            )));
        }
    }
    Ok(unbound)
}

/// Pre- and post-script writes merged in order, so the caller sees one net set of
/// changes: a later unset erases an earlier set and vice versa.
#[derive(Debug, Default)]
struct VarWrites {
    sets: BTreeMap<String, String>,
    unsets: Vec<String>,
}

impl VarWrites {
    fn record(&mut self, sets: &BTreeMap<String, String>, unsets: &[String]) {
        for (key, value) in sets {
            self.unsets.retain(|u| u != key);
            self.sets.insert(key.clone(), value.clone());
        }
        for key in unsets {
            self.sets.remove(key);
            if !self.unsets.iter().any(|u| u == key) {
                self.unsets.push(key.clone());
            }
        }
    }

    /// A declared secret's value never leaves the runner — the caller gets the
    /// name so it can report "prod.token was updated" and nothing else. A value
    /// that quotes an already-resolved secret is held back the same way, so
    /// copying a secret into another variable cannot launder it into a file.
    fn split_secrets(
        self,
        vars: &VarFrame,
    ) -> (BTreeMap<String, String>, Vec<String>, Vec<String>) {
        let mut plain = BTreeMap::new();
        let mut secret = Vec::new();
        for (key, value) in self.sets {
            if vars.secrets.contains_key(&key) || crate::redact::scrub(&value) != value {
                secret.push(key);
            } else {
                plain.insert(key, value);
            }
        }
        (plain, self.unsets, secret)
    }
}

/// The one place an environment name becomes a [`VarFrame`]. Every entry point —
/// suite runs, single runs, the desktop app — goes through it, so the GUI and the
/// CLI cannot drift into resolving variables differently.
pub fn env_frame(ws: &Path, env: Option<&str>) -> CoreResult<VarFrame> {
    let Some(name) = env else {
        return Ok(VarFrame::default());
    };
    let listed = workspace::list_env_docs(ws)?;
    let found = listed
        .items
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {name}")))?;
    Ok(VarFrame::from_doc(&found))
}

fn apply_var_writes(vars: &mut VarFrame, sets: &BTreeMap<String, String>, unsets: &[String]) {
    for (k, v) in sets {
        vars.set(k.clone(), v.clone());
    }
    for k in unsets {
        vars.remove(k);
    }
}

pub fn suite_paths(
    ws: &Path,
    collection_slug: &str,
    filter: Option<&str>,
) -> CoreResult<Vec<String>> {
    let tree = collection::list_tree(ws)?;
    let node = tree
        .collections
        .into_iter()
        .find(|c| c.slug == collection_slug)
        .ok_or_else(|| CoreError::NotFound(format!("unknown collection: {collection_slug}")))?;
    let mut out: Vec<String> = Vec::new();
    collect_paths(&node.requests, &node.folders, &mut out);
    let prefix = filter.map(|f| f.trim_matches('/').to_string());
    if let Some(prefix) = prefix.filter(|p| !p.is_empty()) {
        out.retain(|p| p.starts_with(&format!("{prefix}/")));
        if out.is_empty() {
            return Err(CoreError::NotFound(format!(
                "no requests under folder: {prefix}"
            )));
        }
    }
    Ok(out)
}

fn collect_paths(
    requests: &[collection::RequestSummary],
    folders: &[collection::FolderNode],
    out: &mut Vec<String>,
) {
    for request in requests {
        out.push(request.path.clone());
    }
    for folder in folders {
        collect_paths(&folder.requests, &folder.folders, out);
    }
}
