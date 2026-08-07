pub mod assertions;
pub mod body;
pub mod bundle;
pub mod capability;
pub mod collection;
pub mod curl;
pub mod error;
pub mod git;
pub mod git_sync;
pub mod github_auth;
pub mod grpc;
pub mod grpc_format;
pub mod http_format;
pub mod interpolate;
pub mod openapi;
pub mod postman;
pub mod redact;
pub mod remote;
pub mod request;
pub mod review;
pub mod runner;
pub mod sample;
pub mod scan;
pub mod script;
pub mod secrets;
pub mod share;
pub mod stream;
pub mod stream_format;
pub mod text_format;
pub mod workspace;

pub use body::{Body, FormDataRow, FormRow, RawLanguage};
pub use capability::{
    AllowAll, Decision, EnvVarStore, HostPolicy, LayeredSecrets, MemorySecrets, NoSecrets,
    SecretStore, SecretWriter, StrictPolicy, VarSource,
};
pub use collection::{SavedMessage, SavedRequest, SavedStream};
pub use curl::{from_curl, to_curl};
pub use error::{CoreError, CoreResult};
pub use redact::Redactor;
pub use remote::{RemoteFetch, RemoteOrigin, RemoteReview, RemoteSource};
pub use runner::{LocalVar, RunReport, Runner, StepResult, VarFrame};
pub use scan::Finding;
pub use secrets::LocalStore;
pub use stream::{
    Direction, MqttOptions, Outgoing, Payload, SseOptions, StreamEvent, StreamHandle, StreamKind,
    StreamLimits, StreamRegistry, StreamSpec, StreamStatus, Subscription, WsOptions,
};
pub use workspace::{EnvDoc, Environment, VarDef};

/// Chooses the rustls crypto provider, once, for the whole process.
///
/// The dependency graph legitimately links both providers: `reqwest` and
/// `tokio-tungstenite` ask for `ring`, while `rumqttc`'s `use-rustls` pulls
/// `tokio-rustls` with its defaults, which turn on `aws-lc-rs`. Feature
/// unification cannot be talked out of that from here, and rustls refuses to
/// guess between two — it panics at the first handshake instead. Deciding here
/// rather than in each binary is what keeps the CLI, the desktop app, the tests
/// and any future consumer from each having to remember.
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // `Err` means another caller won the race, which is just as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
