use crate::body::{self, Body};
use crate::capability::{Decision, HostPolicy};
use crate::error::{CoreError, CoreResult};
use crate::interpolate;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpec {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Body::is_none")]
    pub body: Body,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Root that `formdata` and `binary` file paths resolve against. Bodies read
    /// files, so the send path carries the workspace explicitly instead of
    /// reaching for a global — a spec with no root cannot read any file at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    #[default]
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    Apikey {
        key: String,
        value: String,
        placement: String,
    },
    /// Auth the request did not ask for: a collection-wide default it inherited.
    /// It is applied like the auth it wraps, except that a `{{variable}}` nothing
    /// has set yet drops it instead of failing the send — the request that
    /// *produces* that variable inherits the default too, and has to be sendable.
    Inherited {
        auth: Box<Auth>,
    },
}

impl Auth {
    /// The auth that decides what goes on the wire, with the inherited wrapper
    /// peeled off.
    pub fn effective(&self) -> &Auth {
        match self {
            Auth::Inherited { auth } => auth.effective(),
            other => other,
        }
    }

    pub fn is_inherited(&self) -> bool {
        matches!(self, Auth::Inherited { .. })
    }

    pub fn inherited(auth: Auth) -> Auth {
        match auth {
            Auth::None => Auth::None,
            Auth::Inherited { auth } => Auth::Inherited { auth },
            other => Auth::Inherited {
                auth: Box::new(other),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResponseData {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub binary: bool,
    pub duration_ms: u128,
    pub size_bytes: usize,
}

/// How long a single request may take before Mándalo gives up. Named so the
/// timeout message can quote the number the client is actually enforcing.
pub const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Sent on every request that does not set its own `User-Agent`.
///
/// reqwest sends none at all, and a client with no user agent is not merely
/// unidentified — GitHub closes the connection on one, which made every request
/// to github.com fail with `connection closed before message completed` and took
/// the update check down with it. A request that sets its own header still wins.
pub const USER_AGENT: &str = concat!("mandalo/", env!("CARGO_PKG_VERSION"));

/// reqwest reports a refused connection as a chain whose outer message says only
/// "error sending request"; the actionable part (ECONNREFUSED, a rejected
/// certificate) is further down, so the whole chain is collected.
fn causes(error: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut chain = vec![error.to_string()];
    let mut source = error.source();
    while let Some(inner) = source {
        chain.push(inner.to_string());
        source = inner.source();
    }
    chain
}

fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    causes(error).pop().unwrap_or_default()
}

/// Whether a chain ends in a rejected certificate. Matching on the text is the
/// only way: reqwest hands back the rustls failure as an opaque boxed error, so
/// there is no type left to match on.
fn is_certificate_failure(chain: &str) -> bool {
    let lower = chain.to_ascii_lowercase();
    [
        "invalid peer certificate",
        "certificate verify failed",
        "unknownissuer",
        "unknown issuer",
        "self-signed certificate",
        "self signed certificate",
        "certificate has expired",
        "certificateexpired",
        "notvalidforname",
        "bad certificate",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn target_of(error: &reqwest::Error) -> Option<String> {
    let url = error.url()?;
    let host = url.host_str()?;
    Some(match url.port_or_known_default() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// Names the host and says what actually happened. Behind a TLS-inspecting proxy
/// or a private CA every send fails here, and the rustls chain on its own reads
/// as noise nobody can act on.
fn certificate_error(target: &str, cause: &str) -> CoreError {
    CoreError::Tls(format!(
        "the certificate for {target} could not be verified: {cause} — Mándalo trusts the operating system's certificate store, so a private or TLS-inspecting authority has to be installed there"
    ))
}

fn transport_error(error: reqwest::Error) -> CoreError {
    if error.is_timeout() {
        let target = error
            .url()
            .map(|u| format!(" from {u}"))
            .unwrap_or_default();
        return CoreError::Network(format!(
            "no response{target} within {REQUEST_TIMEOUT_SECS}s — Mándalo's request timeout"
        ));
    }
    let chain = causes(&error);
    // Before `is_connect`: reqwest reports a failed handshake as a connect error
    // too, and "could not connect" is the wrong story to tell about it.
    if is_certificate_failure(&chain.join(": ")) {
        return certificate_error(
            &target_of(&error).unwrap_or_else(|| "the server".to_string()),
            chain.last().map(String::as_str).unwrap_or_default(),
        );
    }
    if error.is_connect() {
        let target = target_of(&error).unwrap_or_else(|| "the server".to_string());
        return CoreError::Network(format!(
            "could not connect to {target}: {}",
            root_cause(&error)
        ));
    }
    CoreError::Network(error.to_string())
}

/// Redirects are followed by hand, never by reqwest: the host policy has to run
/// again on every hop, and reqwest only strips `Authorization` across origins —
/// a custom `X-Api-Key` would travel to wherever the first server pointed.
pub fn client() -> CoreResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    crate::install_crypto_provider();
    let built = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| CoreError::Network(e.to_string()))?;
    Ok(CLIENT.get_or_init(|| built))
}

pub const MAX_REDIRECT_HOPS: usize = 10;

/// The one place a url is checked against the host policy. Every egress path —
/// the first request, every redirect hop, gRPC — asks this, so a policy cannot
/// be enforced in one place and forgotten in another.
pub async fn guard_url(policy: &dyn HostPolicy, url: &str) -> CoreResult<()> {
    if !policy.enforces() {
        return Ok(());
    }
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| CoreError::Request(format!("invalid url {url:?}: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| CoreError::Request(format!("url has no host: {url}")))?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);

    let addresses: Vec<std::net::IpAddr> = match host.parse::<std::net::IpAddr>() {
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
        match policy.allow(&host, &ip) {
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

fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn interpolate_json(
    value: &serde_json::Value,
    vars: &BTreeMap<String, String>,
) -> CoreResult<serde_json::Value> {
    use serde_json::Value;
    match value {
        Value::String(s) => Ok(Value::String(interpolate::apply(s, vars)?)),
        Value::Array(items) => items
            .iter()
            .map(|v| interpolate_json(v, vars))
            .collect::<CoreResult<Vec<_>>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(interpolate::apply(k, vars)?, interpolate_json(v, vars)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

enum Payload {
    Empty,
    Bytes(Vec<u8>),
    Multipart(reqwest::multipart::Form),
}

pub(crate) fn graphql_payload(
    query: &str,
    variables: &str,
    vars: &BTreeMap<String, String>,
) -> CoreResult<Vec<u8>> {
    let query = interpolate::apply(query, vars)?;
    let variables: serde_json::Value = if variables.trim().is_empty() {
        serde_json::json!({})
    } else {
        let parsed = serde_json::from_str(variables)
            .map_err(|e| CoreError::Parse(format!("invalid graphql variables JSON: {e}")))?;
        interpolate_json(&parsed, vars)?
    };
    Ok(
        serde_json::json!({ "query": query, "variables": variables })
            .to_string()
            .into_bytes(),
    )
}

fn multipart_form(
    rows: &[body::FormDataRow],
    spec: &RequestSpec,
) -> CoreResult<reqwest::multipart::Form> {
    let vars = &spec.vars;
    let mut form = reqwest::multipart::Form::new();
    for row in rows.iter().filter(|r| r.enabled) {
        let key = interpolate::apply(&row.key, vars)?;
        if !row.is_file() {
            form = form.text(key, interpolate::apply(&row.value, vars)?);
            continue;
        }
        for path in &row.files {
            let file = body::read_file(
                spec.workspace.as_deref(),
                path,
                row.content_type.as_deref(),
                vars,
            )
            .map_err(|e| CoreError::Request(format!("form-data field {key:?}: {}", e.message())))?;
            let part = reqwest::multipart::Part::bytes(file.bytes)
                .file_name(file.file_name)
                .mime_str(&file.content_type)
                .map_err(|e| {
                    CoreError::Request(format!(
                        "form-data field {key:?}: invalid content type {:?}: {e}",
                        file.content_type
                    ))
                })?;
            form = form.part(key.clone(), part);
        }
    }
    Ok(form)
}

fn payload(spec: &RequestSpec) -> CoreResult<(Payload, Option<String>)> {
    let vars = &spec.vars;
    Ok(match &spec.body {
        Body::None => (Payload::Empty, None),
        Body::Raw { language, text } => (
            Payload::Bytes(interpolate::apply(text, vars)?.into_bytes()),
            Some(language.content_type().to_string()),
        ),
        Body::Urlencoded { rows } => (
            Payload::Bytes(body::encode_urlencoded(rows, vars)?),
            Some("application/x-www-form-urlencoded".to_string()),
        ),
        Body::Formdata { rows } => (Payload::Multipart(multipart_form(rows, spec)?), None),
        Body::Binary { file, content_type } => {
            let file = body::read_file(
                spec.workspace.as_deref(),
                file,
                content_type.as_deref(),
                vars,
            )?;
            (Payload::Bytes(file.bytes), Some(file.content_type))
        }
        Body::Graphql { query, variables } => (
            Payload::Bytes(graphql_payload(query, variables, vars)?),
            Some("application/json".to_string()),
        ),
    })
}

/// Interpolates the auth values, or reports that this auth sends nothing.
/// Inherited auth whose variables are not set yet resolves to `None`: the
/// request that produces the token carries the same default and still has to go
/// out. A request's own auth keeps failing loud.
pub(crate) fn resolve_auth(
    auth: &Auth,
    vars: &BTreeMap<String, String>,
) -> CoreResult<Option<Auth>> {
    if let Auth::Inherited { auth } = auth {
        return match resolve_auth(auth, vars) {
            Err(e) if e.code() == "E_UNRESOLVED_VAR" => Ok(None),
            other => other,
        };
    }
    Ok(match auth {
        Auth::None => None,
        Auth::Bearer { token } => Some(Auth::Bearer {
            token: interpolate::apply(token, vars)?,
        }),
        Auth::Basic { username, password } => Some(Auth::Basic {
            username: interpolate::apply(username, vars)?,
            password: interpolate::apply(password, vars)?,
        }),
        Auth::Apikey {
            key,
            value,
            placement,
        } => Some(Auth::Apikey {
            key: interpolate::apply(key, vars)?,
            value: interpolate::apply(value, vars)?,
            placement: placement.clone(),
        }),
        Auth::Inherited { .. } => unreachable!("handled above"),
    })
}

pub fn build(client: &reqwest::Client, spec: &RequestSpec) -> CoreResult<reqwest::Request> {
    let vars = &spec.vars;
    let url = interpolate::apply(&spec.url, vars)?;

    let mut headers = Vec::with_capacity(spec.headers.len());
    for (k, v) in &spec.headers {
        headers.push((interpolate::apply(k, vars)?, interpolate::apply(v, vars)?));
    }

    match spec.kind.as_str() {
        "http" => {}
        "graphql" => {
            if !matches!(spec.body, Body::Graphql { .. }) {
                return Err(CoreError::Request(
                    "graphql request is missing the graphql body".to_string(),
                ));
            }
        }
        other => {
            return Err(CoreError::Unsupported(format!(
                "unsupported request kind: {other}"
            )))
        }
    }

    let method = match spec.body {
        Body::Graphql { .. } => reqwest::Method::POST,
        _ => spec
            .method
            .to_uppercase()
            .parse()
            .map_err(|_| CoreError::Request(format!("invalid method: {}", spec.method)))?,
    };
    let (payload, content_type) = payload(spec)?;

    let (auth_headers, auth_query) = auth_parts(resolve_auth(&spec.auth, vars)?)?;
    // Whatever name the auth lands on wins outright: a hand-written header of the
    // same name would otherwise be sent alongside it, and the server picks.
    for (name, _) in &auth_headers {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    }
    headers.extend(auth_headers);
    // A user Content-Type cannot survive multipart: it would arrive without the
    // boundary reqwest generates, and the server could not parse a single part.
    if matches!(payload, Payload::Multipart(_)) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
    }
    let user_set_content_type = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));

    let mut req = client.request(method, &url);
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    if !auth_query.is_empty() {
        req = req.query(&auth_query);
    }
    if let Some(ct) = content_type {
        if !user_set_content_type {
            req = req.header("Content-Type", ct);
        }
    }
    match payload {
        Payload::Empty => {}
        Payload::Bytes(bytes) => req = req.body(bytes),
        Payload::Multipart(form) => req = req.multipart(form),
    }

    req.build().map_err(|e| CoreError::Request(e.to_string()))
}

pub(crate) type AuthParts = (Vec<(String, String)>, Vec<(String, String)>);

pub(crate) fn auth_parts(auth: Option<Auth>) -> CoreResult<AuthParts> {
    Ok(match auth {
        None => (Vec::new(), Vec::new()),
        Some(Auth::Bearer { token }) => (
            vec![("Authorization".to_string(), format!("Bearer {token}"))],
            Vec::new(),
        ),
        Some(Auth::Basic { username, password }) => (
            vec![(
                "Authorization".to_string(),
                format!("Basic {}", BASE64.encode(format!("{username}:{password}"))),
            )],
            Vec::new(),
        ),
        Some(Auth::Apikey {
            key,
            value,
            placement,
        }) => match placement.as_str() {
            "header" => (vec![(key, value)], Vec::new()),
            "query" => (Vec::new(), vec![(key, value)]),
            other => {
                return Err(CoreError::Unsupported(format!(
                    "unsupported apikey placement: {other}"
                )))
            }
        },
        Some(other) => {
            return Err(CoreError::Unsupported(format!(
                "unsupported auth: {other:?}"
            )))
        }
    })
}

pub async fn send_request(spec: RequestSpec, policy: &dyn HostPolicy) -> CoreResult<ResponseData> {
    let client = client()?;
    let req = build(client, &spec)?;
    guard_url(policy, req.url().as_str()).await?;
    send_built(client, req, policy).await
}

fn redirect_target(response: &reqwest::Response) -> CoreResult<Option<reqwest::Url>> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
        return Ok(None);
    };
    let location = location.to_str().map_err(|_| {
        CoreError::Request("the server redirected to a location that is not text".to_string())
    })?;
    response
        .url()
        .join(location)
        .map(Some)
        .map_err(|e| CoreError::Request(format!("the server redirected to {location:?}: {e}")))
}

/// A redirect is a new request to a new host, so it is treated as one: the host
/// policy runs again, and crossing an origin drops every header the collection
/// set. reqwest only ever strips `Authorization`, which leaves an `X-Api-Key`
/// travelling to wherever the first server pointed.
fn follow(mut next: reqwest::Request, to: reqwest::Url, status: u16) -> reqwest::Request {
    let drops_body = matches!(status, 301..=303);
    if !same_origin(next.url(), &to) {
        next.headers_mut().clear();
    }
    if drops_body && next.method() != reqwest::Method::GET {
        *next.method_mut() = reqwest::Method::GET;
        *next.body_mut() = None;
        next.headers_mut().remove(reqwest::header::CONTENT_TYPE);
        next.headers_mut().remove(reqwest::header::CONTENT_LENGTH);
    }
    *next.url_mut() = to;
    next
}

pub async fn send_built(
    client: &reqwest::Client,
    req: reqwest::Request,
    policy: &dyn HostPolicy,
) -> CoreResult<ResponseData> {
    let started = Instant::now();
    let mut current = req;
    let mut hops = 0usize;
    let resp = loop {
        let repeatable = current.try_clone();
        let response = client.execute(current).await.map_err(transport_error)?;
        let Some(to) = redirect_target(&response)? else {
            break response;
        };
        hops += 1;
        if hops > MAX_REDIRECT_HOPS {
            return Err(CoreError::Request(format!(
                "the server kept redirecting — gave up after {MAX_REDIRECT_HOPS} hops"
            )));
        }
        guard_url(policy, to.as_str()).await?;
        let previous = repeatable.ok_or_else(|| {
            CoreError::Request(
                "this request was redirected but its body can only be sent once — point the request at the final url instead".to_string(),
            )
        })?;
        current = follow(previous, to, response.status().as_u16());
    };
    let status = resp.status();
    let status_text = resp
        .extensions()
        .get::<hyper::ext::ReasonPhrase>()
        .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned())
        .or_else(|| status.canonical_reason().map(str::to_string))
        .unwrap_or_default();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let bytes = resp.bytes().await.map_err(transport_error)?;
    let size_bytes = bytes.len();
    let (body, binary) = match String::from_utf8(bytes.to_vec()) {
        Ok(text) => (text, false),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    };
    Ok(ResponseData {
        status: status.as_u16(),
        status_text,
        headers,
        body,
        binary,
        size_bytes,
        duration_ms: started.elapsed().as_millis(),
    })
}

pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FetchedDocument {
    /// Where the fetch actually landed, redirects included — an import dialog has
    /// to name the host it is about to trust, not the one that was typed.
    pub url: String,
    pub content_type: Option<String>,
    pub bytes: usize,
    pub text: String,
    /// What the final hop answered. `fetch_document` only ever returns a success
    /// here; `fetch_response` hands back the failures too, so a caller that has
    /// to tell "no such repository" from "not yours to see" can read the status
    /// instead of matching on the text of an error.
    #[serde(default = "ok_status")]
    pub status: u16,
}

fn ok_status() -> u16 {
    200
}

fn status_label(status: reqwest::StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("{} {reason}", status.as_u16()),
        None => status.as_u16().to_string(),
    }
}

/// Fetches a document a user is about to import. A url typed into a dialog is
/// the one request an attacker gets to aim, so it goes out through the same host
/// policy as every other egress — on the first url and on every redirect hop —
/// and stops at [`MAX_IMPORT_BYTES`] instead of buffering whatever arrives.
pub async fn fetch_document(url: &str, policy: &dyn HostPolicy) -> CoreResult<FetchedDocument> {
    fetch_document_with(url, policy, &[]).await
}

/// [`fetch_document`] with extra request headers — the one thing a document
/// fetch sometimes needs and a saved request never does. GitHub's API, for
/// instance, refuses a caller with no `user-agent`.
pub async fn fetch_document_with(
    url: &str,
    policy: &dyn HostPolicy,
    headers: &[(&str, &str)],
) -> CoreResult<FetchedDocument> {
    let document = fetch_response(url, policy, headers).await?;
    if !(200..300).contains(&document.status) {
        return Err(CoreError::Request(format!(
            "{} answered {}, so there is no document to import",
            document.url,
            status_label(
                reqwest::StatusCode::from_u16(document.status)
                    .unwrap_or(reqwest::StatusCode::BAD_GATEWAY)
            )
        )));
    }
    Ok(document)
}

/// The same guarded, bounded fetch, with the status handed back rather than
/// raised. Everything else — the host policy on every hop, the size cap, the
/// http/https-only rule — is identical, because it is the same code.
pub async fn fetch_response(
    url: &str,
    policy: &dyn HostPolicy,
    headers: &[(&str, &str)],
) -> CoreResult<FetchedDocument> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| CoreError::Request(format!("{url} is not a url Mándalo can fetch: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(CoreError::Unsupported(format!(
                "{url} is a {other} url — an import can only be fetched over http or https"
            )))
        }
    }

    let client = client()?;
    guard_url(policy, parsed.as_str()).await?;
    let mut builder = client.get(parsed);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut current = builder
        .build()
        .map_err(|e| CoreError::Request(e.to_string()))?;
    let mut hops = 0usize;
    let mut response = loop {
        let repeatable = current.try_clone();
        let response = client.execute(current).await.map_err(transport_error)?;
        let Some(to) = redirect_target(&response)? else {
            break response;
        };
        hops += 1;
        if hops > MAX_REDIRECT_HOPS {
            return Err(CoreError::Request(format!(
                "the server kept redirecting — gave up after {MAX_REDIRECT_HOPS} hops"
            )));
        }
        guard_url(policy, to.as_str()).await?;
        let previous = repeatable.ok_or_else(|| {
            CoreError::Request("this request was redirected but cannot be sent again".to_string())
        })?;
        current = follow(previous, to, response.status().as_u16());
    };

    let final_url = response.url().to_string();
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if let Some(declared) = response.content_length() {
        if declared > MAX_IMPORT_BYTES as u64 {
            return Err(CoreError::Request(format!(
                "{final_url} is {declared} bytes, over the {MAX_IMPORT_BYTES} byte limit for an import"
            )));
        }
    }

    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| CoreError::Network(e.to_string()))?
    {
        if bytes.len() + chunk.len() > MAX_IMPORT_BYTES {
            return Err(CoreError::Request(format!(
                "{final_url} sent more than the {MAX_IMPORT_BYTES} byte limit for an import"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }

    let size_bytes = bytes.len();
    let text = String::from_utf8(bytes).map_err(|e| {
        CoreError::Parse(format!(
            "{final_url} is not text — an import document has to be valid UTF-8, and byte {} is not",
            e.utf8_error().valid_up_to()
        ))
    })?;
    Ok(FetchedDocument {
        url: final_url,
        content_type,
        bytes: size_bytes,
        text,
        status: status.as_u16(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: serde_json::Value) -> RequestSpec {
        serde_json::from_value(json).unwrap()
    }

    fn build_ok(json: serde_json::Value) -> reqwest::Request {
        build(client().unwrap(), &spec(json)).unwrap()
    }

    fn build_err(json: serde_json::Value) -> String {
        build(client().unwrap(), &spec(json))
            .unwrap_err()
            .to_string()
    }

    fn header<'a>(req: &'a reqwest::Request, name: &str) -> &'a str {
        req.headers().get(name).unwrap().to_str().unwrap()
    }

    fn body_str(req: &reqwest::Request) -> String {
        String::from_utf8(req.body().unwrap().as_bytes().unwrap().to_vec()).unwrap()
    }

    #[test]
    fn bearer_auth_sets_authorization_header() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "get", "url": "https://x.dev/a",
            "auth": {"type": "bearer", "token": "{{t}}"},
            "vars": {"t": "secret"}
        }));
        assert_eq!(header(&req, "authorization"), "Bearer secret");
    }

    #[test]
    fn basic_auth_sets_base64_credentials() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "basic", "username": "user", "password": "pass"}
        }));
        assert_eq!(header(&req, "authorization"), "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn apikey_header_placement() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "apikey", "key": "X-Api-Key", "value": "{{k}}", "placement": "header"},
            "vars": {"k": "abc"}
        }));
        assert_eq!(header(&req, "x-api-key"), "abc");
    }

    #[test]
    fn apikey_query_placement_appends_param() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev/search?q=1",
            "auth": {"type": "apikey", "key": "api_key", "value": "abc", "placement": "query"}
        }));
        assert_eq!(req.url().as_str(), "https://x.dev/search?q=1&api_key=abc");
    }

    #[test]
    fn apikey_unknown_placement_fails() {
        let err = build_err(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "auth": {"type": "apikey", "key": "k", "value": "v", "placement": "cookie"}
        }));
        assert!(err.contains("unsupported apikey placement: cookie"));
    }

    #[test]
    fn graphql_forces_post_json_body() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "GET", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "query { user(id: {{id}}) { name } }", "variables": "{\"limit\": 5}"},
            "vars": {"id": "7"}
        }));
        assert_eq!(req.method(), reqwest::Method::POST);
        assert_eq!(header(&req, "content-type"), "application/json");
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        assert_eq!(body["query"], "query { user(id: 7) { name } }");
        assert_eq!(body["variables"], serde_json::json!({"limit": 5}));
    }

    #[test]
    fn graphql_empty_variables_becomes_empty_object() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        }));
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        assert_eq!(body["variables"], serde_json::json!({}));
    }

    #[test]
    fn graphql_invalid_variables_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": "not json"}
        }));
        assert!(err.contains("invalid graphql variables JSON"));
    }

    #[test]
    fn graphql_without_body_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql"
        }));
        assert!(err.contains("missing the graphql body"));
    }

    #[test]
    fn interpolates_url_headers_and_body() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "POST", "url": "{{base}}/users",
            "headers": [["X-{{suffix}}", "v-{{suffix}}"]],
            "body": {"mode": "raw", "language": "text", "text": "hello {{name}}"},
            "vars": {"base": "https://x.dev", "suffix": "trace", "name": "nova"}
        }));
        assert_eq!(req.url().as_str(), "https://x.dev/users");
        assert_eq!(header(&req, "x-trace"), "v-trace");
        assert_eq!(body_str(&req), "hello nova");
    }

    #[test]
    fn unresolved_var_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "http", "method": "GET", "url": "{{base}}/users"
        }));
        assert!(err.contains("unresolved variable: base"));
    }

    #[test]
    fn unsupported_kind_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "soap", "method": "GET", "url": "https://x.dev"
        }));
        assert!(err.contains("unsupported request kind: soap"));
    }

    #[test]
    fn graphql_variable_value_cannot_inject_json_keys() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": "{\"q\":\"{{term}}\"}"},
            "vars": {"term": "x\", \"isAdmin\": true, \"z\": \""}
        }));
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        let variables = body["variables"].as_object().unwrap();
        assert_eq!(variables.len(), 1);
        assert_eq!(variables["q"], "x\", \"isAdmin\": true, \"z\": \"");
    }

    #[test]
    fn graphql_interpolates_nested_keys_and_values() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "body": {
                "mode": "graphql",
                "query": "{ ping }",
                "variables": "{\"filter\": {\"{{field}}\": [\"{{v}}\", 3]}, \"n\": 7}"
            },
            "vars": {"field": "status", "v": "open"}
        }));
        let body: serde_json::Value = serde_json::from_str(&body_str(&req)).unwrap();
        assert_eq!(
            body["variables"],
            serde_json::json!({"filter": {"status": ["open", 3]}, "n": 7})
        );
    }

    #[test]
    fn user_content_type_wins_over_graphql_default() {
        let req = build_ok(serde_json::json!({
            "kind": "graphql", "method": "POST", "url": "https://x.dev/graphql",
            "headers": [["content-type", "application/graphql-response+json"]],
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        }));
        let values: Vec<_> = req.headers().get_all("content-type").iter().collect();
        assert_eq!(values, vec!["application/graphql-response+json"]);
    }

    #[test]
    fn bearer_auth_replaces_user_authorization_header() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "headers": [["Authorization", "stale"]],
            "auth": {"type": "bearer", "token": "fresh"}
        }));
        let values: Vec<_> = req.headers().get_all("authorization").iter().collect();
        assert_eq!(values, vec!["Bearer fresh"]);
    }

    #[test]
    fn basic_auth_replaces_user_authorization_header() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "headers": [["authorization", "stale"]],
            "auth": {"type": "basic", "username": "u", "password": "p"}
        }));
        let values: Vec<_> = req.headers().get_all("authorization").iter().collect();
        assert_eq!(values, vec!["Basic dTpw"]);
    }

    #[test]
    fn user_authorization_header_kept_without_bearer_or_basic() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev",
            "headers": [["Authorization", "custom scheme"]]
        }));
        let values: Vec<_> = req.headers().get_all("authorization").iter().collect();
        assert_eq!(values, vec!["custom scheme"]);
    }

    #[test]
    fn each_mode_sets_its_own_content_type() {
        for (body, expected) in [
            (
                serde_json::json!({"mode": "raw", "language": "json", "text": "{}"}),
                "application/json",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "xml", "text": "<a/>"}),
                "application/xml",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "html", "text": "<p>"}),
                "text/html",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "javascript", "text": "1"}),
                "application/javascript",
            ),
            (
                serde_json::json!({"mode": "raw", "language": "text", "text": "hola"}),
                "text/plain",
            ),
            (
                serde_json::json!({"mode": "urlencoded", "rows": [{"key": "a", "value": "1"}]}),
                "application/x-www-form-urlencoded",
            ),
            (
                serde_json::json!({"mode": "graphql", "query": "{ ping }"}),
                "application/json",
            ),
        ] {
            let req = build_ok(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev", "body": body
            }));
            assert_eq!(header(&req, "content-type"), expected, "{body}");
        }
    }

    #[test]
    fn a_user_content_type_wins_in_every_mode_and_is_never_duplicated() {
        for body in [
            serde_json::json!({"mode": "raw", "language": "json", "text": "{}"}),
            serde_json::json!({"mode": "urlencoded", "rows": [{"key": "a", "value": "1"}]}),
            serde_json::json!({"mode": "graphql", "query": "{ ping }"}),
        ] {
            let req = build_ok(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev",
                "headers": [["Content-Type", "application/vnd.acme+json"]],
                "body": body
            }));
            let values: Vec<_> = req.headers().get_all("content-type").iter().collect();
            assert_eq!(values, vec!["application/vnd.acme+json"], "{body}");
        }
    }

    #[test]
    fn an_empty_body_sends_no_content_type() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev"
        }));
        assert!(req.headers().get("content-type").is_none());
        assert!(req.body().is_none());
    }

    #[test]
    fn urlencoded_body_is_form_encoded_and_skips_disabled_rows() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "POST", "url": "https://x.dev",
            "body": {"mode": "urlencoded", "rows": [
                {"key": "user", "value": "{{who}} lovelace"},
                {"key": "debug", "value": "1", "enabled": false},
                {"key": "q", "value": "a&b"}
            ]},
            "vars": {"who": "ada"}
        }));
        assert_eq!(body_str(&req), "user=ada+lovelace&q=a%26b");
    }

    #[test]
    fn a_graphql_body_forces_post_whatever_the_method_says() {
        let req = build_ok(serde_json::json!({
            "kind": "http", "method": "GET", "url": "https://x.dev/graphql",
            "body": {"mode": "graphql", "query": "{ ping }", "variables": ""}
        }));
        assert_eq!(req.method(), reqwest::Method::POST);
    }

    #[test]
    fn a_file_body_without_a_workspace_cannot_read_anything() {
        for body in [
            serde_json::json!({"mode": "binary", "file": "files/a.png"}),
            serde_json::json!({"mode": "formdata", "rows": [{"key": "a", "files": ["files/a.png"]}]}),
        ] {
            let err = build_err(serde_json::json!({
                "kind": "http", "method": "POST", "url": "https://x.dev", "body": body
            }));
            assert!(err.contains("no workspace root"), "{err}");
        }
    }

    #[test]
    fn shared_client_is_reused() {
        assert!(std::ptr::eq(client().unwrap(), client().unwrap()));
    }

    #[test]
    fn invalid_method_fails_loud() {
        let err = build_err(serde_json::json!({
            "kind": "http", "method": "GE T", "url": "https://x.dev"
        }));
        assert!(err.contains("invalid method"));
    }

    #[test]
    fn a_certificate_failure_names_the_host_and_says_it_is_the_certificate() {
        let error = certificate_error(
            "api.internal:443",
            "invalid peer certificate: UnknownIssuer",
        );
        assert_eq!(error.code(), "E_TLS");
        let text = error.to_string();
        assert!(
            text.starts_with("the certificate for api.internal:443 could not be verified: "),
            "{text}"
        );
        assert!(text.contains("UnknownIssuer"), "{text}");
        assert!(
            text.contains("operating system's certificate store"),
            "{text}"
        );
    }

    #[test]
    fn certificate_chains_are_told_apart_from_every_other_transport_failure() {
        for chain in [
            "error sending request: client error (Connect): invalid peer certificate: UnknownIssuer",
            "invalid peer certificate: Expired",
            "invalid peer certificate: NotValidForName",
            "certificate verify failed: self-signed certificate in certificate chain",
        ] {
            assert!(is_certificate_failure(chain), "{chain}");
        }
        for chain in [
            "error sending request: tcp connect error: Connection refused (os error 61)",
            "operation timed out",
            "dns error: failed to lookup address information: nodename nor servname provided",
            "connection closed before message completed",
        ] {
            assert!(!is_certificate_failure(chain), "{chain}");
        }
    }

    /// A port the OS handed out and released refuses immediately, so the connect
    /// path runs without a server and without a fixed port the CI box may use.
    fn refused_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[tokio::test]
    async fn a_refused_connection_names_the_host_and_the_reason() {
        let port = refused_port();
        let spec = RequestSpec {
            kind: "http".to_string(),
            method: "GET".to_string(),
            url: format!("http://127.0.0.1:{port}/health"),
            ..RequestSpec::default()
        };
        let failure = send_request(spec, &crate::AllowAll).await.unwrap_err();
        assert_eq!(failure.code(), "E_NETWORK");
        let error = failure.to_string();
        assert!(
            error.contains(&format!("could not connect to 127.0.0.1:{port}")),
            "{error}"
        );
        assert!(
            error.to_lowercase().contains("refused"),
            "the root cause has to survive: {error}"
        );
    }
}
