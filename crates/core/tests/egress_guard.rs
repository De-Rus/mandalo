use mandalo_core::capability::{Decision, HostPolicy};
use mandalo_core::request::{self, RequestSpec};
use mandalo_core::{AllowAll, StrictPolicy};
use mandalo_testkit::MockApi;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting {
    calls: AtomicUsize,
    inner: StrictPolicy,
}

impl Counting {
    fn allowing(hosts: &[&str]) -> Self {
        Counting {
            calls: AtomicUsize::new(0),
            inner: StrictPolicy::allowing(hosts),
        }
    }
}

impl HostPolicy for Counting {
    fn allow(&self, host: &str, ip: &IpAddr) -> Decision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.allow(host, ip)
    }
}

fn spec(url: &str, headers: Vec<(String, String)>) -> RequestSpec {
    RequestSpec {
        kind: "http".to_string(),
        method: "GET".to_string(),
        url: url.to_string(),
        headers,
        ..RequestSpec::default()
    }
}

fn escaped(raw: &str) -> String {
    raw.replace('%', "%25")
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace('?', "%3F")
        .replace('=', "%3D")
        .replace('&', "%26")
}

#[tokio::test]
async fn a_redirect_to_a_denied_host_is_refused() {
    let mock = MockApi::start().await;
    let policy = Counting::allowing(&["localhost"]);
    let allowed = format!(
        "http://localhost:{}/redirect-to?url={}",
        mock.addr().port(),
        escaped(&mock.url("/get"))
    );

    let error = request::send_request(spec(&allowed, Vec::new()), &policy)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_HOST_DENIED", "{error}");
    assert!(
        policy.calls.load(Ordering::SeqCst) >= 2,
        "the policy has to run again on the hop, not only on the first url"
    );
}

#[tokio::test]
async fn a_redirect_across_origins_drops_every_header_the_collection_set() {
    let first = MockApi::start().await;
    let second = MockApi::start().await;
    let hop = format!(
        "{}?url={}",
        first.url("/redirect-to"),
        escaped(&second.url("/headers/echo"))
    );

    let response = request::send_request(
        spec(
            &hop,
            vec![("X-Api-Key".to_string(), "s3cr3t-api-key".to_string())],
        ),
        &AllowAll,
    )
    .await
    .unwrap();

    assert_eq!(response.status, 200, "{}", response.body);
    assert!(
        !response.body.to_ascii_lowercase().contains("x-api-key"),
        "the api key travelled to the second host: {}",
        response.body
    );
}

#[tokio::test]
async fn a_same_origin_redirect_keeps_the_headers() {
    let mock = MockApi::start().await;
    let hop = format!(
        "{}?url={}",
        mock.url("/redirect-to"),
        escaped("/headers/echo")
    );

    let response = request::send_request(
        spec(&hop, vec![("X-Trace".to_string(), "on".to_string())]),
        &AllowAll,
    )
    .await
    .unwrap();

    assert_eq!(response.status, 200, "{}", response.body);
    assert!(
        response.body.to_ascii_lowercase().contains("x-trace"),
        "{}",
        response.body
    );
}

#[tokio::test]
async fn a_long_redirect_chain_gives_up_instead_of_following_forever() {
    let mock = MockApi::start().await;
    let error = request::send_request(spec(&mock.url("/redirect/10"), Vec::new()), &AllowAll)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("kept redirecting"), "{error}");

    let ok = request::send_request(spec(&mock.url("/redirect/3"), Vec::new()), &AllowAll)
        .await
        .unwrap();
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn the_policy_is_asked_before_the_first_byte_leaves() {
    let mock = MockApi::start().await;
    let policy = Counting::allowing(&[]);
    let error = request::send_request(spec(&mock.url("/get"), Vec::new()), &policy)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "E_HOST_DENIED", "{error}");
    assert_eq!(mock.received_requests().len(), 0);
}
