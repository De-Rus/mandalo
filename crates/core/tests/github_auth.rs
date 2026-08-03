use mandalo_core::capability::MemorySecrets;
use mandalo_core::github_auth::{
    poll_device_flow, poll_device_flow_once, start_device_flow_at, store_token_in, whoami_at,
    DeviceCode, GitHubUser, PollOutcome,
};
use mandalo_core::{redact, CoreError};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

struct Github {
    base: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Github {
    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

/// A scripted GitHub: one connection per queued answer, served in order, with the
/// raw request text kept so a test can prove what went on the wire.
fn github(script: Vec<(u16, &'static str)>) -> Github {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for (status, body) in script {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let Some(raw) = read_request(&mut stream) else {
                return;
            };
            log.lock().unwrap().push(raw);
            let response = format!(
                "HTTP/1.1 {status} S\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    Github {
        base: format!("http://{addr}"),
        seen,
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<String> {
    let mut buf = [0u8; 4096];
    let mut raw = Vec::new();
    let head_end = loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
    let length: usize = head
        .split("content-length:")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while raw.len() < head_end + length {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
    }
    Some(String::from_utf8_lossy(&raw).to_string())
}

const DEVICE_OK: &str = r#"{"device_code":"dev-code-fixture-9f8a","user_code":"WDJB-MJHT","verification_uri":"https://github.com/login/device","expires_in":900,"interval":5}"#;

const DEVICE_FAST: &str = r#"{"device_code":"dev-code-fixture-9f8a","user_code":"WDJB-MJHT","verification_uri":"https://github.com/login/device","expires_in":900,"interval":0}"#;

const TOKEN_OK: &str = r#"{"access_token":"gho_16C7e42F292c6912E7710c838347Ae178B4a","token_type":"bearer","scope":"repo"}"#;

const TOKEN_VALUE: &str = "gho_16C7e42F292c6912E7710c838347Ae178B4a";

async fn started(server: &Github) -> (DeviceCode, mandalo_core::github_auth::DeviceHandle) {
    start_device_flow_at(&server.base, "Ov23liTESTCLIENTID", &["repo"])
        .await
        .unwrap()
}

#[tokio::test]
async fn the_happy_path_asks_for_a_code_then_gets_a_token() {
    let server = github(vec![(200, DEVICE_OK), (200, TOKEN_OK)]);

    let (code, handle) = started(&server).await;
    assert_eq!(code.user_code, "WDJB-MJHT");
    assert_eq!(code.verification_uri, "https://github.com/login/device");
    assert_eq!(code.expires_in, 900);
    assert_eq!(code.interval, 5);
    assert_eq!(handle.interval_secs(), 5);
    assert!(!handle.expired());

    let outcome = poll_device_flow_once(&handle).await.unwrap();
    assert_eq!(outcome, PollOutcome::Token(TOKEN_VALUE.to_string()));

    let sent = server.requests();
    assert!(sent[0].starts_with("POST /login/device/code HTTP/1.1"));
    assert!(sent[0].to_lowercase().contains("accept: application/json"));
    assert!(sent[0].to_lowercase().contains("user-agent: mandalo/"));
    assert!(sent[0].contains("client_id=Ov23liTESTCLIENTID"));
    assert!(sent[0].contains("scope=repo"));

    assert!(sent[1].starts_with("POST /login/oauth/access_token HTTP/1.1"));
    assert!(sent[1].to_lowercase().contains("accept: application/json"));
    assert!(sent[1].contains("device_code=dev-code-fixture-9f8a"));
    assert!(sent[1].contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
}

#[tokio::test]
async fn authorization_pending_keeps_polling_until_the_token_arrives() {
    let server = github(vec![
        (200, DEVICE_OK),
        (200, r#"{"error":"authorization_pending"}"#),
        (200, TOKEN_OK),
    ]);
    let (_, handle) = started(&server).await;

    assert_eq!(
        poll_device_flow_once(&handle).await.unwrap(),
        PollOutcome::Pending
    );
    assert_eq!(
        handle.interval_secs(),
        5,
        "pending must not change the pace"
    );
    assert_eq!(
        poll_device_flow_once(&handle).await.unwrap(),
        PollOutcome::Token(TOKEN_VALUE.to_string())
    );
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn slow_down_widens_the_interval_github_asked_for() {
    let server = github(vec![
        (200, DEVICE_OK),
        (200, r#"{"error":"slow_down","interval":10}"#),
        (200, TOKEN_OK),
    ]);
    let (_, handle) = started(&server).await;
    assert_eq!(handle.interval_secs(), 5);

    assert_eq!(
        poll_device_flow_once(&handle).await.unwrap(),
        PollOutcome::Pending
    );
    assert_eq!(handle.interval_secs(), 10);

    assert_eq!(
        poll_device_flow_once(&handle).await.unwrap(),
        PollOutcome::Token(TOKEN_VALUE.to_string())
    );
    assert_eq!(handle.interval_secs(), 10);
}

#[tokio::test]
async fn slow_down_without_a_number_still_backs_off() {
    let server = github(vec![(200, DEVICE_OK), (200, r#"{"error":"slow_down"}"#)]);
    let (_, handle) = started(&server).await;

    assert_eq!(
        poll_device_flow_once(&handle).await.unwrap(),
        PollOutcome::Pending
    );
    assert!(handle.interval_secs() > 5);
    assert_eq!(handle.interval_secs(), 10);
}

#[tokio::test]
async fn the_polling_loop_honours_pending_and_slow_down_and_resolves() {
    let server = github(vec![
        (200, DEVICE_FAST),
        (200, r#"{"error":"authorization_pending"}"#),
        (200, r#"{"error":"slow_down","interval":0}"#),
        (200, TOKEN_OK),
    ]);
    let (_, handle) = started(&server).await;

    let token = poll_device_flow(&handle).await.unwrap();
    assert_eq!(token, TOKEN_VALUE);
    assert_eq!(server.requests().len(), 4);
}

#[tokio::test]
async fn an_expired_token_is_a_loud_terminal_error() {
    let server = github(vec![
        (200, DEVICE_OK),
        (
            200,
            r#"{"error":"expired_token","error_description":"raw slug leaked"}"#,
        ),
    ]);
    let (_, handle) = started(&server).await;

    let error = poll_device_flow_once(&handle).await.unwrap_err();
    assert_eq!(error.code(), "E_REQUEST");
    assert!(error.to_string().contains("expired"));
    assert!(!error.to_string().contains("expired_token"));
}

#[tokio::test]
async fn access_denied_is_a_loud_terminal_error() {
    let server = github(vec![
        (200, DEVICE_OK),
        (200, r#"{"error":"access_denied"}"#),
    ]);
    let (_, handle) = started(&server).await;

    let error = poll_device_flow_once(&handle).await.unwrap_err();
    assert_eq!(error.code(), "E_REQUEST");
    assert!(error.to_string().contains("cancelled"));
}

#[tokio::test]
async fn an_incorrect_device_code_is_a_loud_terminal_error() {
    let server = github(vec![
        (200, DEVICE_OK),
        (400, r#"{"error":"incorrect_device_code"}"#),
    ]);
    let (_, handle) = started(&server).await;

    let error = poll_device_flow_once(&handle).await.unwrap_err();
    assert_eq!(error.code(), "E_REQUEST");
    assert!(error.to_string().contains("start again"));
}

#[tokio::test]
async fn a_disabled_device_flow_says_so_at_the_first_step() {
    let server = github(vec![(200, r#"{"error":"device_flow_disabled"}"#)]);

    let error = start_device_flow_at(&server.base, "Ov23liTESTCLIENTID", &["repo"])
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_UNSUPPORTED");
    assert!(error.to_string().contains("device flow"));
}

#[tokio::test]
async fn a_malformed_answer_fails_to_parse_instead_of_being_guessed_at() {
    let server = github(vec![(200, "<html>a captive portal</html>")]);

    let error = start_device_flow_at(&server.base, "Ov23liTESTCLIENTID", &["repo"])
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(error.to_string().contains("not JSON"));
}

#[tokio::test]
async fn an_answer_missing_the_device_code_is_a_parse_error() {
    let server = github(vec![(
        200,
        r#"{"user_code":"WDJB-MJHT","verification_uri":"x","expires_in":900,"interval":5}"#,
    )]);

    let error = start_device_flow_at(&server.base, "Ov23liTESTCLIENTID", &["repo"])
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
    assert!(error.to_string().contains("device_code"));
}

#[tokio::test]
async fn whoami_parses_the_identity_and_sends_the_headers_github_demands() {
    let server = github(vec![(
        200,
        r#"{"login":"drus","name":"Dani Rus","avatar_url":"https://avatars.githubusercontent.com/u/1","id":1}"#,
    )]);

    let user = whoami_at(&server.base, TOKEN_VALUE).await.unwrap();
    assert_eq!(
        user,
        GitHubUser {
            login: "drus".to_string(),
            name: Some("Dani Rus".to_string()),
            avatar_url: Some("https://avatars.githubusercontent.com/u/1".to_string()),
        }
    );

    let sent = &server.requests()[0];
    assert!(sent.starts_with("GET /user HTTP/1.1"));
    let lower = sent.to_ascii_lowercase();
    assert!(lower.contains("accept: application/vnd.github+json"));
    assert!(lower.contains("x-github-api-version: 2022-11-28"));
    assert!(lower.contains("user-agent: mandalo/"));
    assert!(sent.contains(&format!("Bearer {TOKEN_VALUE}")));
}

#[tokio::test]
async fn whoami_tolerates_a_user_with_no_display_name() {
    let server = github(vec![(200, r#"{"login":"ghost","name":null}"#)]);

    let user = whoami_at(&server.base, TOKEN_VALUE).await.unwrap();
    assert_eq!(user.login, "ghost");
    assert_eq!(user.name, None);
    assert_eq!(user.avatar_url, None);
}

#[tokio::test]
async fn a_bad_personal_access_token_is_rejected_before_it_is_ever_stored() {
    let server = github(vec![(401, r#"{"message":"Bad credentials"}"#)]);
    let store = MemorySecrets::new();

    let error = whoami_at(&server.base, "ghp_obviously_wrong")
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_REQUEST");
    assert!(error.to_string().contains("rejected"));

    assert_eq!(
        mandalo_core::github_auth::stored_token_in(&store).unwrap(),
        None
    );
}

#[tokio::test]
async fn whoami_reports_a_refusal_with_githubs_own_words() {
    let server = github(vec![(
        403,
        r#"{"message":"API rate limit exceeded for user"}"#,
    )]);

    let error = whoami_at(&server.base, TOKEN_VALUE).await.unwrap_err();
    assert_eq!(error.code(), "E_REQUEST");
    assert!(error.to_string().contains("rate limit"));
}

#[tokio::test]
async fn whoami_refuses_an_answer_that_is_not_a_user() {
    let server = github(vec![(200, r#"{"documentation_url":"https://docs"}"#)]);

    let error = whoami_at(&server.base, TOKEN_VALUE).await.unwrap_err();
    assert_eq!(error.code(), "E_PARSE");
}

#[tokio::test]
async fn the_token_never_reaches_an_error_message() {
    let server = github(vec![(200, DEVICE_OK), (200, TOKEN_OK)]);
    let (_, handle) = started(&server).await;
    let token = match poll_device_flow_once(&handle).await.unwrap() {
        PollOutcome::Token(token) => token,
        other => panic!("expected a token, got {other:?}"),
    };

    for error in CoreError::all_variants(&format!("git push failed carrying {token}")) {
        let shown = error.to_string();
        assert!(
            !shown.contains(&token),
            "{} leaked the token: {shown}",
            error.code()
        );
        assert!(shown.contains("[redacted:github.token]"));
    }
}

#[test]
fn a_stored_token_is_redacted_from_every_error_variant() {
    let store = MemorySecrets::new();
    let pasted = "ghp_pasted_by_hand_7c3e91f2";
    store_token_in(&store, pasted).unwrap();

    for error in CoreError::all_variants(&format!("cannot push: {pasted}")) {
        assert!(!error.to_string().contains(pasted));
    }
    assert!(!redact::scrub(pasted).contains(pasted));
}

/// Talks to the real github.com. It proves two things nothing mocked can: that the
/// shipped client id is registered, and that "Enable Device Flow" is actually
/// ticked on the OAuth App. It stops at the handshake — completing the grant needs
/// a human at github.com/login/device — so it never earns or stores a token.
///
/// Off unless `MANDALO_GITHUB_LIVE=1`, so CI stays offline and deterministic.
#[tokio::test]
async fn the_shipped_client_id_really_starts_a_device_flow() {
    if std::env::var("MANDALO_GITHUB_LIVE").as_deref() != Ok("1") {
        return;
    }
    let (code, handle) = start_device_flow_at(
        mandalo_core::github_auth::OAUTH_BASE,
        mandalo_core::github_auth::MANDALO_CLIENT_ID,
        mandalo_core::github_auth::DEFAULT_SCOPES,
    )
    .await
    .expect("github refused the device-code request — is Device Flow enabled on the OAuth App?");

    assert!(!code.user_code.is_empty());
    assert!(code
        .verification_uri
        .starts_with("https://github.com/login/device"));
    assert!(code.expires_in >= 300);
    assert!(code.interval >= 1);
    assert_eq!(handle.interval_secs(), code.interval);
}

#[test]
fn the_ipc_payloads_are_camel_case() {
    let code = DeviceCode {
        user_code: "WDJB-MJHT".to_string(),
        verification_uri: "https://github.com/login/device".to_string(),
        expires_in: 900,
        interval: 5,
    };
    assert_eq!(
        serde_json::to_value(&code).unwrap(),
        serde_json::json!({
            "userCode": "WDJB-MJHT",
            "verificationUri": "https://github.com/login/device",
            "expiresIn": 900,
            "interval": 5
        })
    );

    let user = GitHubUser {
        login: "drus".to_string(),
        name: Some("Dani Rus".to_string()),
        avatar_url: Some("https://avatars.githubusercontent.com/u/1".to_string()),
    };
    assert_eq!(
        serde_json::to_value(&user).unwrap(),
        serde_json::json!({
            "login": "drus",
            "name": "Dani Rus",
            "avatarUrl": "https://avatars.githubusercontent.com/u/1"
        })
    );
}
