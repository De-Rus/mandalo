use mandalo_core::capability::{Decision, HostPolicy};
use mandalo_core::request::{self, MAX_IMPORT_BYTES};
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

fn escaped(raw: &str) -> String {
    raw.replace('%', "%25")
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace('?', "%3F")
        .replace('=', "%3D")
        .replace('&', "%26")
}

#[tokio::test]
async fn a_fetched_document_arrives_with_its_type_and_byte_count() {
    let mock = MockApi::start().await;
    let document = request::fetch_document(&mock.url("/text"), &AllowAll)
        .await
        .unwrap();

    assert_eq!(document.text, "hola");
    assert_eq!(document.bytes, 4);
    assert_eq!(document.url, mock.url("/text"));
    assert!(
        document
            .content_type
            .as_deref()
            .unwrap_or_default()
            .starts_with("text/plain"),
        "{:?}",
        document.content_type
    );
}

#[tokio::test]
async fn a_document_served_without_a_content_type_still_imports() {
    let mock = MockApi::start().await;
    let document = request::fetch_document(&mock.url("/empty"), &AllowAll)
        .await
        .unwrap();

    assert_eq!(document.content_type, None);
    assert_eq!(document.bytes, 0);
    assert_eq!(document.text, "");
}

#[tokio::test]
async fn a_redirect_chain_reports_the_url_it_landed_on() {
    let mock = MockApi::start().await;
    let document = request::fetch_document(&mock.url("/redirect/3"), &AllowAll)
        .await
        .unwrap();

    assert_eq!(document.url, mock.url("/get"));
    assert!(
        document.text.contains("\"path\":\"/get\""),
        "{}",
        document.text
    );
}

#[tokio::test]
async fn a_redirect_to_a_blocked_host_is_refused() {
    let mock = MockApi::start().await;
    let policy = Counting::allowing(&["localhost"]);
    let allowed = format!(
        "http://localhost:{}/redirect-to?url={}",
        mock.addr().port(),
        escaped(&mock.url("/get"))
    );

    let error = request::fetch_document(&allowed, &policy)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_HOST_DENIED", "{error}");
    assert!(
        policy.calls.load(Ordering::SeqCst) >= 2,
        "the policy has to run again on the hop, not only on the first url"
    );
    assert!(
        mock.requests_to("/get").is_empty(),
        "the blocked hop was fetched anyway"
    );
}

#[tokio::test]
async fn a_long_redirect_chain_gives_up_instead_of_following_forever() {
    let mock = MockApi::start().await;
    let error = request::fetch_document(&mock.url("/redirect/10"), &AllowAll)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("kept redirecting"), "{error}");
}

#[tokio::test]
async fn a_body_that_declares_more_than_the_limit_is_refused_before_it_is_read() {
    let mock = MockApi::start().await;
    let url = mock.url("/stream?mb=65");
    let error = request::fetch_document(&url, &AllowAll).await.unwrap_err();

    assert!(
        error.to_string().contains(&format!(
            "over the {MAX_IMPORT_BYTES} byte limit for an import"
        )),
        "{error}"
    );
}

#[tokio::test]
async fn a_body_that_keeps_arriving_past_the_limit_is_cut_off() {
    let mock = MockApi::start().await;
    let url = mock.url("/stream?mb=65&chunked=1");
    let error = request::fetch_document(&url, &AllowAll).await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains(&format!("more than the {MAX_IMPORT_BYTES} byte limit")),
        "{error}"
    );
}

#[tokio::test]
async fn a_body_within_the_limit_is_read_whole() {
    let mock = MockApi::start().await;
    let document = request::fetch_document(&mock.url("/stream?mb=2&chunked=1"), &AllowAll)
        .await
        .unwrap();
    assert_eq!(document.bytes, 2 * 1024 * 1024);
}

#[tokio::test]
async fn a_url_that_is_not_http_is_refused_by_name() {
    let error = request::fetch_document("ftp://files.acme.com/openapi.json", &AllowAll)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_UNSUPPORTED", "{error}");
    assert!(
        error.to_string().contains("is a ftp url"),
        "the scheme has to be named: {error}"
    );

    for scheme in ["file:///etc/passwd", "data:text/plain,hola"] {
        let error = request::fetch_document(scheme, &AllowAll)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "E_UNSUPPORTED", "{scheme}: {error}");
    }
}

#[tokio::test]
async fn a_body_that_is_not_utf8_fails_instead_of_arriving_mangled() {
    let mock = MockApi::start().await;
    let error = request::fetch_document(&mock.url("/binary"), &AllowAll)
        .await
        .unwrap_err();

    assert_eq!(error.code(), "E_PARSE", "{error}");
    assert!(error.to_string().contains("valid UTF-8"), "{error}");
}

#[tokio::test]
async fn an_error_page_is_an_error_not_a_document() {
    let mock = MockApi::start().await;
    let error = request::fetch_document(&mock.url("/status/404"), &AllowAll)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("answered 404 Not Found"),
        "{error}"
    );
    assert!(
        error.to_string().contains("no document to import"),
        "{error}"
    );
}
