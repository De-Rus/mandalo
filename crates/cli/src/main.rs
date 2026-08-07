use clap::{Parser, Subcommand};
use mandalo_cli::listen;
use mandalo_cli::report::{json_request, render, to_json, DataReporter, JsonSend, Reporter};
use mandalo_cli::style::Style;
use mandalo_core::git_sync::{self, Auth, SyncAction, SyncOutcome, SyncPlan, SyncSelection};
use mandalo_core::remote;
use mandalo_core::request;
use mandalo_core::runner::{self, Runner, StepResult, VarFrame};
use mandalo_core::script::{self, ScriptContext, ScriptRequest};
use mandalo_core::stream::{self, Outgoing, StreamKind, StreamSpec, Subscription};
use mandalo_core::{
    bundle, collection, curl, git, github_auth, interpolate, openapi, postman, redact, scan,
    workspace, AllowAll, CoreError, CoreResult, EnvVarStore, HostPolicy, LayeredSecrets,
    LocalStore, Redactor, SecretStore, StrictPolicy, VarSource,
};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "mandalo",
    version,
    about = "Mándalo — run API collections from a terminal or from CI",
    disable_help_subcommand = true,
    after_help = "Quickstart:\n  \
        mandalo init                                  create a workspace here\n  \
        mandalo ls                                    list collections and requests\n  \
        mandalo send api auth/login.http#0 --env dev  send one request\n  \
        mandalo run api --env dev                     run a whole collection\n  \
        pbpaste | mandalo env set-secret dev token    store a secret in the keychain"
)]
struct Cli {
    /// Workspace directory — absolute or relative to the current directory — or a
    /// registered workspace id (defaults to the active workspace)
    #[arg(long, global = true, value_name = "PATH")]
    workspace: Option<String>,

    /// Refuse requests to loopback, private, link-local and cloud metadata hosts
    #[arg(long, global = true)]
    strict_network: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every request in a collection and report the results
    #[command(after_help = "Examples:\n  \
        mandalo run api --env staging\n  \
        mandalo run api --folder auth --fail-fast\n  \
        mandalo run api --env ci --reporter junit > results.xml")]
    Run {
        /// Collection slug
        collection: String,
        /// Only run requests under this folder path
        #[arg(long, value_name = "PATH")]
        folder: Option<String>,
        /// Environment to load variables from
        #[arg(long, value_name = "NAME")]
        env: Option<String>,
        #[arg(long, value_enum, default_value_t = Reporter::Pretty)]
        reporter: Reporter,
        /// Stop at the first failing request
        #[arg(long)]
        fail_fast: bool,
    },
    /// Send one saved request and print the response
    #[command(after_help = "Examples:\n  \
        mandalo send api auth/login.http#0 --env dev\n  \
        mandalo send api 'auth/login.http#Sign in' --env dev\n  \
        mandalo send api users/list.http --reporter json | jq .response.status")]
    Send {
        /// Collection slug
        collection: String,
        /// Request path inside the collection, e.g. auth/login.http#0
        request_path: String,
        #[arg(long, value_name = "NAME")]
        env: Option<String>,
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
    /// Open a realtime stream and print every event until it closes
    #[command(after_help = "Examples:\n  \
        mandalo listen wss://echo.example/socket --send hello --max 5\n  \
        mandalo listen https://api.example/events --timeout 30\n  \
        mandalo listen mqtt://broker.example --topic 'sensors/#' --qos 1\n  \
        mandalo listen mock chat/socket.ws#0 --message subscribe\n  \
        mandalo listen mock sensors/room.mqtt#Sensors --env local")]
    Listen {
        /// A url, or the collection slug when a saved stream follows it
        url: String,
        /// Saved stream inside that collection, e.g. chat/socket.ws#0
        request_path: Option<String>,
        /// Name of a saved message to send once connected, repeatable
        #[arg(long = "message", value_name = "NAME")]
        messages: Vec<String>,
        /// Protocol, when the scheme does not say it (mqtt over ws, for one)
        #[arg(long, value_enum)]
        kind: Option<ListenKind>,
        /// Extra header, repeatable: --header "name: value"
        #[arg(long = "header", value_name = "K:V")]
        headers: Vec<String>,
        /// Topic to subscribe to, repeatable (mqtt)
        #[arg(long = "topic", value_name = "FILTER")]
        topics: Vec<String>,
        /// Quality of service for the subscriptions and for --send (mqtt)
        #[arg(long, default_value_t = 0)]
        qos: u8,
        /// Text to send once connected, repeatable; needs --topic on mqtt
        #[arg(long = "send", value_name = "TEXT")]
        send: Vec<String>,
        /// Environment to read {{vars}} from
        #[arg(long, value_name = "NAME")]
        env: Option<String>,
        /// Print one JSON event per line instead of a readable log
        #[arg(long)]
        json: bool,
        /// Stop after this many incoming messages
        #[arg(long, value_name = "N")]
        max: Option<usize>,
        /// Stop after this many seconds
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
        /// Reconnect when the connection drops
        #[arg(long)]
        reconnect: bool,
    },
    /// Read and write environment variables
    #[command(after_help = "Examples:\n  \
        mandalo env list\n  \
        mandalo env set dev baseUrl https://api.dev.example\n  \
        pbpaste | mandalo env set-secret dev token")]
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// Create a workspace here: mandalo.toml, collections/ and environments/
    Init {
        /// Where to create it (defaults to the current directory)
        path: Option<PathBuf>,
        /// Workspace name (defaults to the directory name)
        #[arg(long, value_name = "TEXT")]
        name: Option<String>,
    },
    /// Print the collection tree
    Ls {
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
    /// Import an OpenAPI/Swagger specification, a Postman collection or a Mándalo
    /// bundle, from a file or an http(s) url
    Import { source: String },
    /// Print a saved request as a curl command line
    #[command(after_help = "Examples:\n  \
        mandalo curl api auth/login.http#0 --env dev\n  \
        mandalo curl api users/list.http --env prod | pbcopy")]
    Curl {
        /// Collection slug
        collection: String,
        /// Request path inside the collection, e.g. auth/login.http#0
        request_path: String,
        #[arg(long, value_name = "NAME")]
        env: Option<String>,
    },
    /// Read a curl command line and print the request it describes
    #[command(after_help = "Examples:\n  \
        mandalo from-curl \"curl -X POST https://api.example/users -d '{}'\"\n  \
        pbpaste | mandalo from-curl --reporter json")]
    FromCurl {
        /// The command; read from stdin when it is not given
        command: Option<String>,
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
    /// Export the workspace, or part of it, as a Mándalo bundle or Postman collection
    Export {
        file: PathBuf,
        /// Only this collection, repeatable (defaults to every collection)
        #[arg(long = "collection", value_name = "SLUG")]
        collections: Vec<String>,
        /// Only this folder, repeatable: --folder acme-api:users
        #[arg(long = "folder", value_name = "[SLUG:]PATH")]
        folders: Vec<String>,
        /// Only this request, repeatable: --request acme-api:users/list.http
        #[arg(long = "request", value_name = "[SLUG:]PATH")]
        requests: Vec<String>,
        /// Only this environment, repeatable (defaults to every environment)
        #[arg(long = "env", value_name = "NAME")]
        envs: Vec<String>,
        /// Say "the whole workspace" out loud; refuses to combine with the filters
        #[arg(long)]
        all: bool,
        /// Override `[share] format` for this export: `bundle` or `postman`
        #[arg(long = "format", value_name = "FORMAT", value_parser = ["bundle", "postman"])]
        format: Option<String>,
        /// Write it without asking, for CI
        #[arg(long, short = 'y')]
        yes: bool,
        /// Write it even though the credential scanner found something
        #[arg(long)]
        force: bool,
    },
    /// Look for credential-looking literals in workspace files
    Scan {
        /// Only scan files staged for the next commit
        #[arg(long)]
        staged: bool,
    },
    /// Keep the workspace safe to commit: managed .gitignore, optional pre-commit hook
    GitHygiene {
        /// Also install the pre-commit hook that runs `mandalo scan --staged`
        #[arg(long)]
        install_hook: bool,
    },
    /// Commit the workspace, rebase on what changed, and push it back
    Sync {
        /// The commit message
        #[arg(long, short, value_name = "TEXT", default_value = "Update workspace")]
        message: String,
        /// Only commit this changed file or directory, repeatable
        #[arg(long = "only", value_name = "PATH")]
        only: Vec<String>,
        /// Leave this changed file or directory out, repeatable
        #[arg(long = "except", value_name = "PATH")]
        except: Vec<String>,
        /// Commit and push without asking, for CI
        #[arg(long, short = 'y')]
        yes: bool,
        /// Commit even though the credential scanner found something
        #[arg(long)]
        force: bool,
    },
    /// Open a collection published somewhere else, read-only
    #[command(after_help = "Examples:\n  \
        mandalo open acme/collections                 a public GitHub repository\n  \
        mandalo open acme/collections/apis#next       a subdirectory on a branch\n  \
        mandalo open https://github.com/acme/apis     the url from the address bar\n  \
        mandalo open https://acme.dev/team.json       a single Mándalo bundle\n\n\
        A collection opened this way is read-only and nothing in it runs until you send it.\n\
        For a private repository, `mandalo login` then `mandalo clone` — that gives you a\n\
        workspace you own and can push back to.")]
    Open {
        /// owner/name, a github.com url, or the url of a Mándalo bundle
        source: String,
        /// Where to put it (defaults to a directory named after the repository)
        dest: Option<PathBuf>,
        /// Open it without being asked to confirm the review
        #[arg(long)]
        yes: bool,
        /// Print the review as JSON and open nothing
        #[arg(long)]
        review_only: bool,
        /// Read the repository from somewhere other than GitHub. The test suite
        /// points this at a local fixture so it never touches the network.
        #[arg(long, hide = true, value_name = "URL")]
        github_api: Option<String>,
        #[arg(long, hide = true, value_name = "URL")]
        github_raw: Option<String>,
    },
    /// Save a copy of a read-only remote workspace as one you own
    SaveCopy {
        /// Where to put the copy
        dest: PathBuf,
        /// Name for the new workspace (defaults to the directory name)
        #[arg(long)]
        name: Option<String>,
    },
    /// Copy a workspace from a git remote onto this machine
    Clone {
        /// Remote URL, https or ssh
        url: String,
        /// Where to put it (defaults to the repository name in the current directory)
        dest: Option<PathBuf>,
    },
    /// Sign in to GitHub so collections can be pushed and pulled
    Login {
        /// Read a personal access token from stdin instead of opening a browser
        #[arg(long)]
        with_token: bool,
        /// Ask only for access to public repositories
        #[arg(long)]
        public_only: bool,
        /// Use another OAuth App, for a fork or a self-hosted build
        #[arg(long, value_name = "ID")]
        client_id: Option<String>,
        /// Print the verification URL instead of opening a browser
        #[arg(long)]
        no_browser: bool,
    },
    /// Forget the stored GitHub token
    Logout,
    /// Print the GitHub account the stored token belongs to
    Whoami {
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
enum ListenKind {
    Ws,
    Sse,
    Mqtt,
}

impl From<ListenKind> for StreamKind {
    fn from(kind: ListenKind) -> StreamKind {
        match kind {
            ListenKind::Ws => StreamKind::WebSocket,
            ListenKind::Sse => StreamKind::Sse,
            ListenKind::Mqtt => StreamKind::Mqtt,
        }
    }
}

#[derive(Subcommand)]
enum EnvCommand {
    /// List the environments in the workspace
    List {
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
    /// Print the variables of one environment
    Get { name: String },
    /// Write one shared variable into an environment
    Set {
        name: String,
        key: String,
        value: String,
    },
    /// Store a credential for this machine, read from stdin. It is masked in
    /// every view and never written to a workspace file.
    SetSecret { name: String, key: String },
    /// Store a value for this machine, read from stdin, without masking it —
    /// a URL that points at your own box, not a credential.
    SetLocal { name: String, key: String },
    /// Forget this machine's value for a variable. The declaration stays.
    ClearSecret { name: String, key: String },
}

struct Ctx {
    workspace: PathBuf,
    style: Style,
    redactor: &'static Redactor,
    strict: bool,
    interactive: bool,
}

impl Ctx {
    fn say(&self, text: &str) {
        println!("{}", self.redactor.scrub(text));
    }

    /// The gate every operation that leaves the machine passes through. Without
    /// a terminal to answer there is no consent to assume, so it refuses.
    fn agreed(&self, yes: bool, question: &str) -> CoreResult<bool> {
        if yes {
            return Ok(true);
        }
        if !self.interactive {
            return Err(CoreError::Unsupported(format!(
                "nothing to ask on: {question} — run this with --yes when there is no terminal"
            )));
        }
        print!("{question} [y/N] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|e| CoreError::io("cannot read the answer", e))?;
        Ok(matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "y" | "yes"
        ))
    }

    fn policy(&self) -> Box<dyn HostPolicy> {
        if self.strict {
            Box::new(StrictPolicy::new())
        } else {
            Box::new(AllowAll)
        }
    }

    fn runner(&self) -> CoreResult<Runner<LayeredSecrets, Box<dyn HostPolicy>>> {
        Ok(Runner::new(self.secrets()?, self.policy()).with_workspace(self.workspace.clone()))
    }

    /// Exported variable first, then the values this machine keeps in
    /// `secrets.toml`. Bound to the workspace **id**, so moving or re-cloning
    /// the directory keeps its values attached.
    fn secrets(&self) -> CoreResult<LayeredSecrets> {
        Ok(LayeredSecrets::over(self.local_store()?))
    }

    fn local_store(&self) -> CoreResult<LocalStore> {
        Ok(match workspace::read_manifest(&self.workspace)? {
            Some(manifest) => LocalStore::for_workspace(manifest.id),
            None => LocalStore::unavailable(format!(
                "{} is not a Mándalo workspace yet — run `mandalo init` there",
                self.workspace.display()
            )),
        })
    }
}

/// `--workspace` takes either a registered id or a path. An id is looked up
/// first — ids are minted, paths are typed — and a path is resolved against the
/// current directory, so `--workspace ./ws-example` means what it looks like.
fn resolve_workspace(arg: Option<&str>) -> CoreResult<PathBuf> {
    if let Some(raw) = arg {
        let as_path = Path::new(raw);
        if as_path.is_absolute() {
            if !as_path.is_dir() {
                return Err(CoreError::NotFound(format!(
                    "workspace directory does not exist: {raw}"
                )));
            }
            return Ok(as_path.to_path_buf());
        }
        let registry = workspace::registry_path()?;
        if let Ok(found) = workspace::resolve_workspace(&registry, raw) {
            return Ok(found);
        }
        if as_path.is_dir() {
            return std::fs::canonicalize(as_path).map_err(|e| CoreError::io(as_path.display(), e));
        }
        return Err(CoreError::NotFound(format!(
            "no registered workspace id and no directory named {raw:?} — pass a path, or `mandalo init {raw}` to create one"
        )));
    }
    let registry = workspace::registry_path()?;
    let list = workspace::list_workspaces(&registry, &workspace::default_workspace_path()?)?;
    let active = list
        .items
        .iter()
        .find(|w| w.id == list.active)
        .or_else(|| list.items.first())
        .ok_or_else(|| CoreError::NotFound("there is no workspace yet".to_string()))?;
    Ok(PathBuf::from(&active.path))
}

/// `init` makes the workspace, so it cannot need one to exist first.
///
/// Signing in is what a brand-new user does *before* there is anything to sign in
/// for, so the GitHub commands must not die on "there is no workspace yet".
fn needs_workspace(command: &Command) -> bool {
    !matches!(
        command,
        Command::Init { .. }
            | Command::FromCurl { .. }
            | Command::Login { .. }
            | Command::Logout
            | Command::Whoami { .. }
            | Command::Clone { .. }
            | Command::Open { .. }
            | Command::Listen {
                env: None,
                request_path: None,
                ..
            }
    )
}

fn main() {
    let cli = Cli::parse();
    let style = Style::for_stdout();
    // A command that needs no workspace does not even look for one: resolving the
    // active workspace *creates* the default one on a fresh machine, and
    // `mandalo init ./here` must not leave a stray ~/Mandalo behind.
    let workspace = match needs_workspace(&cli.command) {
        false => PathBuf::from(cli.workspace.clone().unwrap_or_default()),
        true => match resolve_workspace(cli.workspace.as_deref()) {
            Ok(path) => path,
            Err(e) => {
                eprintln!("{}", style.fail(&format!("{} [{}]", e, e.code())));
                std::process::exit(1);
            }
        },
    };
    let ctx = Ctx {
        workspace,
        style,
        redactor: redact::global(),
        strict: cli.strict_network,
        interactive: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
    };

    match dispatch(&ctx, cli.command) {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!(
                "{}",
                ctx.style
                    .fail(&ctx.redactor.scrub(&format!("{} [{}]", e, e.code())))
            );
            std::process::exit(1);
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn dispatch(ctx: &Ctx, command: Command) -> CoreResult<bool> {
    match command {
        Command::Run {
            collection,
            folder,
            env,
            reporter,
            fail_fast,
        } => run_suite(ctx, &collection, folder, env, reporter, fail_fast).await,
        Command::Send {
            collection,
            request_path,
            env,
            reporter,
        } => send_one(ctx, &collection, &request_path, env, reporter).await,
        Command::Listen {
            url,
            request_path,
            messages,
            kind,
            headers,
            topics,
            qos,
            send,
            env,
            json,
            max,
            timeout,
            reconnect,
        } => {
            listen_to(
                ctx,
                ListenRequest {
                    url,
                    request_path,
                    messages,
                    kind,
                    headers,
                    topics,
                    qos,
                    send,
                    env,
                    json,
                    max,
                    timeout,
                    reconnect,
                },
            )
            .await
        }
        Command::Init { path, name } => init(ctx, path.as_deref(), name.as_deref()),
        Command::Env { command } => env_command(ctx, command),
        Command::Ls { reporter } => list_tree(ctx, reporter),
        Command::Import { source } => import(ctx, &source).await,
        Command::Curl {
            collection,
            request_path,
            env,
        } => curl_out(ctx, &collection, &request_path, env),
        Command::FromCurl { command, reporter } => curl_in(ctx, command.as_deref(), reporter),
        Command::Export {
            file,
            collections,
            folders,
            requests,
            envs,
            all,
            format,
            yes,
            force,
        } => export(
            ctx,
            &file,
            ExportFlags {
                collections,
                folders,
                requests,
                envs,
                all,
                format,
            },
            yes,
            force,
        ),
        Command::Scan { staged } => run_scan(ctx, staged),
        Command::GitHygiene { install_hook } => git_hygiene(ctx, install_hook),
        Command::Sync {
            message,
            only,
            except,
            yes,
            force,
        } => sync(
            ctx,
            &message,
            SyncSelection {
                only: (!only.is_empty()).then_some(only),
                except,
            },
            yes,
            force,
        ),
        Command::Open {
            source,
            dest,
            yes,
            review_only,
            github_api,
            github_raw,
        } => {
            let github = remote::Endpoints::github();
            open_remote(
                ctx,
                &source,
                dest.as_deref(),
                yes,
                review_only,
                &remote::Endpoints::at(
                    github_api.unwrap_or(github.api),
                    github_raw.unwrap_or(github.raw),
                ),
            )
            .await
        }
        Command::SaveCopy { dest, name } => save_copy(ctx, &dest, name.as_deref()),
        Command::Clone { url, dest } => clone(ctx, &url, dest.as_deref()),
        Command::Login {
            with_token,
            public_only,
            client_id,
            no_browser,
        } => login(ctx, with_token, public_only, client_id, no_browser).await,
        Command::Logout => logout(ctx),
        Command::Whoami { reporter } => whoami(ctx, reporter).await,
    }
}

/// A stream does not go through the runner, so it has no secret resolution and
/// no host binding to keep a secret from travelling somewhere it was never bound
/// to. Naming a secret is refused instead of quietly sending it anywhere.
fn refuse_secrets(frame: &VarFrame, url: &str, headers: &[(String, String)]) -> CoreResult<()> {
    let mut referenced = interpolate::names(url);
    for (name, value) in headers {
        referenced.extend(interpolate::names(name));
        referenced.extend(interpolate::names(value));
    }
    for name in referenced {
        if frame.secrets.contains_key(&name) {
            return Err(CoreError::Unsupported(format!(
                "{name} is a secret, and `mandalo listen` cannot send secrets yet — pass the value with --header for now"
            )));
        }
    }
    Ok(())
}

struct ListenRequest {
    url: String,
    request_path: Option<String>,
    messages: Vec<String>,
    kind: Option<ListenKind>,
    headers: Vec<String>,
    topics: Vec<String>,
    qos: u8,
    send: Vec<String>,
    env: Option<String>,
    json: bool,
    max: Option<usize>,
    timeout: Option<u64>,
    reconnect: bool,
}

/// A saved stream connects with what the file already says. The flags stay
/// additive on top of it: an extra header, an extra topic, an extra message.
fn saved_spec(
    ctx: &Ctx,
    slug: &str,
    path: &str,
    frame: &VarFrame,
    request: &ListenRequest,
) -> CoreResult<(StreamSpec, Vec<Outgoing>)> {
    let saved = collection::load_request(&ctx.workspace, slug, path)?;
    let stream = saved.stream.as_ref().ok_or_else(|| {
        CoreError::Unsupported(format!(
            "{path} is a {} request, not a stream — send it with `mandalo send`",
            saved.kind
        ))
    })?;
    let mut send: Vec<Outgoing> = Vec::new();
    for name in &request.messages {
        send.push(stream::outgoing_for(stream, name, &frame.vars)?);
    }
    let mut spec = stream::spec_for(&saved, frame.vars.clone())?;
    for raw in &request.headers {
        spec.headers.push(listen::header(raw)?);
    }
    for topic in &request.topics {
        spec.mqtt.subscriptions.push(Subscription {
            topic: topic.clone(),
            qos: request.qos,
        });
    }
    if request.reconnect {
        spec.ws.auto_reconnect = true;
        spec.sse.auto_reconnect = true;
    }
    if let Some(source) = saved
        .scripts
        .pre
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let outcome = script::run_script(
            source,
            ScriptContext {
                vars: frame.script_vars(),
                request_name: saved.name.clone(),
                request: ScriptRequest {
                    method: saved.method.clone(),
                    url: spec.url.clone(),
                    headers: spec.headers.clone(),
                    body: None,
                },
                response: None,
            },
            script::Limits::default(),
        )?;
        for line in &outcome.logs {
            ctx.say(&ctx.style.dim(line));
        }
        for (name, value) in outcome.var_sets {
            spec.vars.insert(name, value);
        }
        for name in &outcome.var_unsets {
            spec.vars.remove(name);
        }
        if let Some(patch) = outcome.request_patch {
            spec.url = patch.url;
            spec.headers = patch.headers;
        }
    }
    send.extend(match spec.kind {
        StreamKind::Sse if !request.send.is_empty() => {
            return Err(CoreError::Stream(SSE_IS_ONE_WAY.to_string()))
        }
        StreamKind::Mqtt => literal_publishes(request)?,
        _ => request.send.iter().map(Outgoing::text).collect(),
    });
    Ok((spec, send))
}

const SSE_IS_ONE_WAY: &str =
    "server-sent events only travel from the server to the client — there is nothing to send";

fn literal_publishes(request: &ListenRequest) -> CoreResult<Vec<Outgoing>> {
    if request.send.is_empty() {
        return Ok(Vec::new());
    }
    let topic = request.topics.first().cloned().ok_or_else(|| {
        CoreError::Stream("an mqtt publish needs a topic — pass --topic".to_string())
    })?;
    Ok(request
        .send
        .iter()
        .map(|text| Outgoing::Publish {
            topic: topic.clone(),
            payload: text.clone(),
            qos: request.qos,
            retain: false,
        })
        .collect())
}

fn url_spec(request: &ListenRequest, frame: VarFrame) -> CoreResult<(StreamSpec, Vec<Outgoing>)> {
    let kind = match request.kind {
        Some(kind) => kind.into(),
        None => listen::kind_from_url(&request.url)?,
    };
    let mut spec = StreamSpec::new(kind, &request.url);
    spec.headers = request
        .headers
        .iter()
        .map(|raw| listen::header(raw))
        .collect::<CoreResult<Vec<_>>>()?;
    spec.vars = frame.vars;
    spec.ws.auto_reconnect = request.reconnect;
    spec.sse.auto_reconnect = request.reconnect;
    spec.mqtt.subscriptions = request
        .topics
        .iter()
        .map(|topic| Subscription {
            topic: topic.clone(),
            qos: request.qos,
        })
        .collect();

    let send = match kind {
        StreamKind::Mqtt => literal_publishes(request)?,
        StreamKind::Sse if !request.send.is_empty() => {
            return Err(CoreError::Stream(SSE_IS_ONE_WAY.to_string()))
        }
        _ => request.send.iter().map(Outgoing::text).collect(),
    };
    Ok((spec, send))
}

async fn listen_to(ctx: &Ctx, request: ListenRequest) -> CoreResult<bool> {
    let frame = load_frame(ctx, request.env.as_deref())?;
    let (spec, send) = match request.request_path.clone() {
        Some(path) => saved_spec(ctx, &request.url, &path, &frame, &request)?,
        None => url_spec(&request, frame.clone())?,
    };
    if !request.messages.is_empty() && request.request_path.is_none() {
        return Err(CoreError::Unsupported(
            "--message names a message saved in a collection — pass the collection and the request path, or use --send".to_string(),
        ));
    }
    refuse_secrets(&frame, &spec.url, &spec.headers)?;

    listen::listen(
        spec,
        Arc::from(ctx.policy()),
        &ctx.style,
        listen::Options {
            max_messages: request.max,
            timeout_secs: request.timeout,
            send,
            json: request.json,
        },
    )
    .await
}

async fn run_suite(
    ctx: &Ctx,
    collection: &str,
    folder: Option<String>,
    env: Option<String>,
    reporter: Reporter,
    fail_fast: bool,
) -> CoreResult<bool> {
    let report = ctx
        .runner()?
        .run_suite_with(
            &ctx.workspace,
            collection,
            folder.as_deref(),
            env.as_deref(),
            fail_fast,
        )
        .await?;
    ctx.say(&render(reporter, &report, &ctx.style)?);
    Ok(report.passed)
}

async fn send_one(
    ctx: &Ctx,
    collection: &str,
    request_path: &str,
    env: Option<String>,
    reporter: DataReporter,
) -> CoreResult<bool> {
    let request = collection::load_request(&ctx.workspace, collection, request_path)?;
    let mut vars = load_frame(ctx, env.as_deref())?;
    let mut step = match ctx.runner()?.run_request(&request, &mut vars).await {
        Ok(step) => step,
        // A transport failure is data for a machine reader, but stays a hard error
        // for a human so the shell still sees a non-zero exit with a readable message.
        Err(e) if reporter == DataReporter::Json => {
            let mut step = StepResult::failed(&request.name, request_path, &e, 0);
            step.method = request.method.clone();
            step.url = request.url.clone();
            step
        }
        Err(e) => return Err(e),
    };
    step.path = request_path.to_string();
    match reporter {
        DataReporter::Pretty => ctx.say(&describe(&step, &ctx.style)),
        DataReporter::Json => ctx.say(&to_json(&JsonSend {
            collection: collection.to_string(),
            env,
            request: json_request(&step),
        })?),
    }
    Ok(step.passed)
}

/// The same seam the runner uses, so `send` resolves variables exactly as `run`
/// does — declarations, this machine's values and every host binding included.
fn load_frame(ctx: &Ctx, env: Option<&str>) -> CoreResult<VarFrame> {
    runner::env_frame(&ctx.workspace, &ctx.secrets()?, env)
}

fn describe(step: &StepResult, style: &Style) -> String {
    let mut out = String::new();
    if let Some(response) = &step.response {
        out.push_str(&format!(
            "{} {}  {}\n",
            style.bold(&response.status.to_string()),
            response.status_text,
            style.dim(&format!(
                "{}ms · {} bytes",
                response.duration_ms, response.size_bytes
            ))
        ));
        for (k, v) in &response.headers {
            out.push_str(&format!("{}: {v}\n", style.dim(k)));
        }
        out.push('\n');
        out.push_str(&response.body);
        out.push('\n');
    }
    if let Some(grpc) = &step.grpc {
        out.push_str(&format!(
            "{}\n{}\n",
            style.dim(&format!("grpc · {}ms", grpc.duration_ms)),
            grpc.body
        ));
    }
    for log in &step.logs {
        out.push_str(&format!("{}\n", style.dim(&format!("log  {log}"))));
    }
    for test in &step.tests {
        out.push_str(&format!(
            "{} {}{}\n",
            if test.passed {
                style.pass("PASS")
            } else {
                style.fail("FAIL")
            },
            test.name,
            test.detail
                .as_ref()
                .map(|d| format!("  ({d})"))
                .unwrap_or_default()
        ));
    }
    for test in &step.script_tests {
        out.push_str(&format!(
            "{} {}{}\n",
            if test.passed {
                style.pass("PASS")
            } else {
                style.fail("FAIL")
            },
            test.name,
            test.error
                .as_ref()
                .map(|d| format!("  ({d})"))
                .unwrap_or_default()
        ));
    }
    for (key, value) in &step.captured {
        out.push_str(&format!(
            "{}\n",
            style.dim(&format!("captured {key}={value}"))
        ));
    }
    out
}

fn env_command(ctx: &Ctx, command: EnvCommand) -> CoreResult<bool> {
    match command {
        EnvCommand::List {
            reporter: DataReporter::Json,
        } => {
            ctx.say(&to_json(&workspace::list_environments(&ctx.workspace)?)?);
            Ok(true)
        }
        EnvCommand::List { .. } => {
            let listed = workspace::list_env_docs(&ctx.workspace)?;
            if listed.items.is_empty() {
                ctx.say("no environments yet");
            }
            for env in &listed.items {
                let local = env.vars.values().filter(|v| !v.is_shared()).count();
                let detail = if local == 0 {
                    format!("{} variables", env.vars.len())
                } else {
                    format!("{} variables · {local} on this machine", env.vars.len())
                };
                ctx.say(&format!("{}  {}", env.name, ctx.style.dim(&detail)));
            }
            for skipped in &listed.skipped {
                ctx.say(&ctx.style.warn(&format!("skipped {skipped}")));
            }
            Ok(true)
        }
        EnvCommand::Get { name } => {
            let doc = workspace::read_env_doc(&ctx.workspace, &name)?
                .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {name}")))?;
            let secrets = ctx.secrets()?;
            for (key, def) in &doc.vars {
                let found = secrets.source(&name, key)?;
                match def {
                    workspace::VarDef::Shared { value } => {
                        // Naming the layer is what keeps precedence from being a
                        // surprise: an exported variable wins, and says so.
                        match found {
                            Some(source) => ctx.say(&format!(
                                "{key}={}",
                                ctx.style.dim(&format!(
                                    "overridden {} · shared value {value}",
                                    describe_source(source)
                                ))
                            )),
                            None => ctx.say(&format!("{key}={value}")),
                        }
                    }
                    workspace::VarDef::Local { hosts } | workspace::VarDef::Secret { hosts } => {
                        let bound = if hosts.is_empty() {
                            "not bound to a host".to_string()
                        } else {
                            format!("for {}", hosts.join(", "))
                        };
                        let kind = if def.is_secret() { "secret" } else { "local" };
                        let held = match found {
                            Some(source) => describe_source(source).to_string(),
                            None => "no value on this machine".to_string(),
                        };
                        ctx.say(&format!(
                            "{key}={}",
                            ctx.style.dim(&format!("{kind} · {bound} · {held}"))
                        ));
                    }
                }
            }
            Ok(true)
        }
        EnvCommand::SetSecret { name, key } => {
            let value = read_secret_from_stdin(&name, &key)?;
            workspace::set_secret(&ctx.workspace, &ctx.local_store()?, &name, &key, &value)?;
            ctx.say(&format!(
                "{name}.{key} stored on this machine; the environment file records only the declaration"
            ));
            Ok(true)
        }
        EnvCommand::SetLocal { name, key } => {
            let value = read_secret_from_stdin(&name, &key)?;
            workspace::set_local(&ctx.workspace, &ctx.local_store()?, &name, &key, &value)?;
            ctx.say(&format!("{name}.{key} stored on this machine"));
            Ok(true)
        }
        EnvCommand::ClearSecret { name, key } => {
            workspace::clear_secret(&ctx.workspace, &ctx.local_store()?, &name, &key)?;
            ctx.say(&format!(
                "{name}.{key} forgotten on this machine; the declaration stays"
            ));
            Ok(true)
        }
        EnvCommand::Set { name, key, value } => {
            let findings = scan::scan_text(Path::new("<value>"), &value);
            if let Some(finding) = findings.first() {
                return Err(CoreError::Secret(format!(
                    "{key} looks like a credential ({}); environments are plain files meant for git — pass it as {} instead",
                    finding.rule,
                    EnvVarStore::variable_name(&name, &key)
                )));
            }
            let listed = workspace::list_environments(&ctx.workspace)?;
            let mut env = listed.items.into_iter().find(|e| e.name == name).unwrap_or(
                workspace::Environment {
                    name: name.clone(),
                    vars: Default::default(),
                },
            );
            env.vars.insert(key.clone(), value);
            workspace::save_environment(&ctx.workspace, &ctx.local_store()?, &env)?;
            ctx.say(&format!("{name}.{key} saved"));
            Ok(true)
        }
    }
}

/// A directory with no `mandalo.toml` is not an empty workspace, it is a typo:
/// printing "no collections yet" for it would hide the mistake.
fn require_workspace(ctx: &Ctx) -> CoreResult<()> {
    if workspace::read_manifest(&ctx.workspace)?.is_some() {
        return Ok(());
    }
    Err(CoreError::NotFound(format!(
        "{} is not a Mándalo workspace — there is no mandalo.toml in it; create one with `mandalo init {}`",
        ctx.workspace.display(),
        ctx.workspace.display()
    )))
}

fn init(ctx: &Ctx, path: Option<&Path>, name: Option<&str>) -> CoreResult<bool> {
    let target = match path {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().map_err(|e| CoreError::io("the current directory", e))?,
    };
    std::fs::create_dir_all(&target).map_err(|e| CoreError::io(target.display(), e))?;
    let absolute =
        std::fs::canonicalize(&target).map_err(|e| CoreError::io(target.display(), e))?;
    let name = match name {
        Some(name) => name.to_string(),
        None => absolute
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Workspace")
            .to_string(),
    };
    let info = workspace::create_workspace(&workspace::registry_path()?, &absolute, &name)?;
    ctx.say(&format!("workspace {} created at {}", info.name, info.path));
    ctx.say(
        &ctx.style
            .dim("add a collection with the app, or drop a .http file into collections/<slug>/"),
    );
    Ok(true)
}

fn list_tree(ctx: &Ctx, reporter: DataReporter) -> CoreResult<bool> {
    require_workspace(ctx)?;
    let tree = collection::list_tree(&ctx.workspace)?;
    if reporter == DataReporter::Json {
        ctx.say(&to_json(&tree)?);
        return Ok(true);
    }
    if tree.collections.is_empty() {
        ctx.say("no collections yet");
    }
    for node in &tree.collections {
        ctx.say(&format!(
            "{}  {}",
            ctx.style.bold(&node.slug),
            ctx.style.dim(&node.name)
        ));
        print_level(ctx, &node.requests, &node.folders, 1);
    }
    for skipped in &tree.skipped {
        ctx.say(&ctx.style.warn(&format!("skipped {skipped}")));
    }
    Ok(true)
}

fn print_level(
    ctx: &Ctx,
    requests: &[collection::RequestSummary],
    folders: &[collection::FolderNode],
    depth: usize,
) {
    let pad = "  ".repeat(depth);
    for request in requests {
        ctx.say(&format!(
            "{pad}{:<7} {}  {}",
            request.method.to_uppercase(),
            request.name,
            ctx.style.dim(&request.path)
        ));
    }
    for folder in folders {
        ctx.say(&format!("{pad}{}/", ctx.style.bold(&folder.name)));
        print_level(ctx, &folder.requests, &folder.folders, depth + 1);
    }
}

async fn import(ctx: &Ctx, source: &str) -> CoreResult<bool> {
    require_workspace(ctx)?;
    let json = if source.starts_with("http://") || source.starts_with("https://") {
        let fetched = request::fetch_document(source, ctx.policy().as_ref()).await?;
        ctx.say(
            &ctx.style
                .dim(&format!("{} bytes from {}", fetched.bytes, fetched.url)),
        );
        fetched.text
    } else {
        let file = Path::new(source);
        std::fs::read_to_string(file).map_err(|e| CoreError::io(file.display(), e))?
    };
    let report = if json.contains("\"mandaloBundle\"") {
        bundle::import(&ctx.workspace, &json)?
    } else if openapi::looks_like_openapi(&json) {
        openapi::import(&ctx.workspace, &json)?
    } else {
        postman::import(&ctx.workspace, &json)?
    };
    ctx.say(&format!(
        "{} requests, {} collections, {} environments imported",
        report.imported, report.collections, report.environments
    ));
    for warning in &report.warnings {
        ctx.say(&ctx.style.warn(warning));
    }
    for skipped in &report.skipped {
        ctx.say(&ctx.style.warn(&format!("skipped {skipped}")));
    }
    ctx.say(&ctx.style.dim(&report.summary));
    Ok(true)
}

/// The same variable frame `send` resolves with, so the printed command carries
/// exactly what would have gone on the wire — an unresolved `{{var}}` stops here
/// rather than being pasted into a shell.
fn curl_out(
    ctx: &Ctx,
    collection: &str,
    request_path: &str,
    env: Option<String>,
) -> CoreResult<bool> {
    require_workspace(ctx)?;
    let saved = collection::load_request(&ctx.workspace, collection, request_path)?;
    let frame = load_frame(ctx, env.as_deref())?;
    let spec = request::RequestSpec {
        kind: saved.kind.clone(),
        method: saved.method.clone(),
        url: saved.url.clone(),
        headers: saved.headers.clone(),
        body: saved.body.clone(),
        auth: saved.auth.clone(),
        vars: frame.vars.clone(),
        workspace: Some(ctx.workspace.clone()),
    };
    ctx.say(&curl::to_curl(&spec)?);
    Ok(true)
}

fn curl_in(ctx: &Ctx, command: Option<&str>, reporter: DataReporter) -> CoreResult<bool> {
    let source = match command {
        Some(command) => command.to_string(),
        None => read_curl_from_stdin()?,
    };
    let spec = curl::from_curl(&source)?;
    match reporter {
        DataReporter::Pretty => ctx.say(&describe_spec(&spec, &ctx.style)),
        DataReporter::Json => ctx.say(&to_json(&spec)?),
    }
    Ok(true)
}

fn read_curl_from_stdin() -> CoreResult<String> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| CoreError::io("cannot read the curl command from stdin", e))?;
    if raw.trim().is_empty() {
        return Err(CoreError::Parse(
            "no curl command arrived on stdin — pipe it in, e.g. `pbpaste | mandalo from-curl`"
                .to_string(),
        ));
    }
    Ok(raw)
}

fn describe_spec(spec: &request::RequestSpec, style: &Style) -> String {
    let mut out = format!("{} {}\n", style.bold(&spec.method), spec.url);
    for (k, v) in &spec.headers {
        out.push_str(&format!("{}: {v}\n", style.dim(k)));
    }
    if let request::Auth::Basic { username, .. } = &spec.auth {
        out.push_str(&format!(
            "{}\n",
            style.dim(&format!("basic auth as {username}"))
        ));
    }
    if !spec.body.is_none() {
        out.push('\n');
        match spec.body.as_text() {
            Some(text) => out.push_str(text),
            None => out.push_str(&format!("<{} body>", spec.body.mode())),
        }
        out.push('\n');
    }
    out
}

struct ExportFlags {
    collections: Vec<String>,
    folders: Vec<String>,
    requests: Vec<String>,
    envs: Vec<String>,
    all: bool,
    format: Option<String>,
}

/// `--folder acme:users` names its collection. A bare `--folder users` is only
/// unambiguous when exactly one `--collection` was given, so that is the only
/// time it is accepted.
fn qualify(entry: &str, collections: &[String]) -> CoreResult<(String, String)> {
    if let Some((slug, path)) = entry.split_once(':') {
        return Ok((slug.to_string(), path.to_string()));
    }
    match collections {
        [only] => Ok((only.clone(), entry.to_string())),
        _ => Err(CoreError::InvalidName(format!(
            "{entry} does not say which collection it is in — write it as <collection>:{entry}"
        ))),
    }
}

fn selection_from(flags: &ExportFlags) -> CoreResult<bundle::ExportSelection> {
    let narrowed = !flags.collections.is_empty()
        || !flags.folders.is_empty()
        || !flags.requests.is_empty()
        || !flags.envs.is_empty();
    if flags.all && narrowed {
        return Err(CoreError::InvalidName(
            "--all exports everything, so it cannot be combined with --collection, --folder, --request or --env".to_string(),
        ));
    }
    let mut chosen: Vec<bundle::CollectionSelection> = flags
        .collections
        .iter()
        .map(bundle::CollectionSelection::whole)
        .collect();
    for (entries, folders) in [(&flags.folders, true), (&flags.requests, false)] {
        for entry in entries {
            let (slug, path) = qualify(entry, &flags.collections)?;
            let at = match chosen.iter().position(|c| c.slug == slug) {
                Some(at) => at,
                None => {
                    chosen.push(bundle::CollectionSelection::whole(slug));
                    chosen.len() - 1
                }
            };
            if folders {
                chosen[at].folders.push(path);
            } else {
                chosen[at].requests.push(path);
            }
        }
    }
    Ok(bundle::ExportSelection {
        collections: (!chosen.is_empty()).then_some(chosen),
        environments: (!flags.envs.is_empty()).then(|| flags.envs.clone()),
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn show_findings(ctx: &Ctx, findings: &[mandalo_core::Finding], where_: &str) {
    for finding in findings {
        ctx.say(&format!(
            "  {} {}:{} {}",
            ctx.style.fail("credential"),
            finding.path.display(),
            finding.line,
            ctx.style
                .dim(&format!("{} · {}", finding.rule, finding.excerpt))
        ));
    }
    if !findings.is_empty() {
        ctx.say(&ctx.style.fail(&format!(
            "{} would be written into {where_} — nothing has been {} yet",
            plural(findings.len(), "credential", "credentials"),
            if where_ == "the commit" {
                "committed"
            } else {
                "written"
            }
        )));
        ctx.say(
            &ctx.style
                .dim("take them out of the files, or repeat the command with --force"),
        );
    }
}

fn export(ctx: &Ctx, file: &Path, flags: ExportFlags, yes: bool, force: bool) -> CoreResult<bool> {
    let selection = selection_from(&flags)?;
    let format = match flags.format.as_deref() {
        Some("postman") => Some(mandalo_core::workspace::ShareFormat::Postman),
        Some("bundle") => Some(mandalo_core::workspace::ShareFormat::Native),
        Some(other) => {
            return Err(CoreError::InvalidName(format!(
                "unknown export format {other:?} — use bundle or postman"
            )));
        }
        None => None,
    };
    let plan = bundle::plan_export_as(&ctx.workspace, &selection, format)?;

    ctx.say(&format!(
        "Export ({}) to {}",
        plan.format,
        ctx.style.bold(&file.display().to_string())
    ));
    ctx.say(&format!(
        "  {} from {}",
        plural(plan.included.request_count, "request", "requests"),
        plural(plan.included.collections.len(), "collection", "collections")
    ));
    for collection in &plan.included.collections {
        ctx.say(&format!(
            "    {}  {}",
            collection.name,
            ctx.style
                .dim(&plural(collection.requests.len(), "request", "requests"))
        ));
    }
    ctx.say(&if plan.included.environments.is_empty() {
        "  no environments".to_string()
    } else {
        format!(
            "  {}: {}",
            plural(
                plan.included.environments.len(),
                "environment",
                "environments"
            ),
            plan.included.environments.join(", ")
        )
    });
    ctx.say(&format!("  {} bytes", plan.bytes));

    if plan.excluded.secret_values > 0 {
        ctx.say(&ctx.style.dim(&format!(
            "  {} are not included — a secret value never lives in the workspace",
            plural(plan.excluded.secret_values, "secret value", "secret values")
        )));
    }
    if plan.excluded.local_values > 0 {
        ctx.say(&ctx.style.dim(&format!(
            "  {} are not included — they belong to this machine only",
            plural(plan.excluded.local_values, "local value", "local values")
        )));
    }
    if !plan.excluded.collections.is_empty() {
        ctx.say(&ctx.style.dim(&format!(
            "  left out: {}",
            plan.excluded.collections.join(", ")
        )));
    }
    if plan.excluded.requests > 0 {
        ctx.say(&ctx.style.dim(&format!(
            "  {} in the chosen collections are not included",
            plural(plan.excluded.requests, "request", "requests")
        )));
    }
    if !plan.excluded.environments.is_empty() {
        ctx.say(&ctx.style.dim(&format!(
            "  environments left out: {}",
            plan.excluded.environments.join(", ")
        )));
    }

    show_findings(ctx, &plan.findings, "the export");
    if plan.blocked && !force {
        return Ok(false);
    }
    if !ctx.agreed(yes, "Write this file?")? {
        ctx.say("nothing was written");
        return Ok(false);
    }
    let receipt =
        bundle::run_export_as(&ctx.workspace, &selection, format, &plan.token, file, force)?;
    ctx.say(&ctx.style.pass(&format!(
        "exported {} to {}",
        plural(receipt.requests, "request", "requests"),
        receipt.path
    )));
    Ok(true)
}

fn git_hygiene(ctx: &Ctx, install_hook: bool) -> CoreResult<bool> {
    let result = git::ensure_git_hygiene(&ctx.workspace)?;
    ctx.say(if result.gitignore_written {
        ".gitignore updated"
    } else {
        ".gitignore already covers the secret side-files"
    });
    if install_hook {
        let path = git::install_precommit_hook(&ctx.workspace)?;
        ctx.say(&format!("pre-commit hook installed at {}", path.display()));
    } else if !result.hook_installed {
        ctx.say(
            &ctx.style
                .dim("no pre-commit hook yet — add one with --install-hook"),
        );
    }
    Ok(true)
}

async fn login(
    ctx: &Ctx,
    with_token: bool,
    public_only: bool,
    client_id: Option<String>,
    no_browser: bool,
) -> CoreResult<bool> {
    let token = if with_token {
        read_token_from_stdin()?
    } else {
        device_flow(ctx, public_only, client_id, no_browser).await?
    };
    let user = github_auth::whoami(&token).await?;
    github_auth::store_token(&token)?;
    ctx.say(&format!(
        "signed in to GitHub as {}",
        ctx.style.bold(&user.login)
    ));
    Ok(true)
}

async fn device_flow(
    ctx: &Ctx,
    public_only: bool,
    client_id: Option<String>,
    no_browser: bool,
) -> CoreResult<String> {
    let id = github_auth::resolve_client_id(client_id.as_deref())?;
    let scopes = if public_only {
        github_auth::PUBLIC_REPO_SCOPES
    } else {
        github_auth::DEFAULT_SCOPES
    };
    let (code, handle) = github_auth::start_device_flow(&id, scopes).await?;
    ctx.say(&format!(
        "open {}\nand enter the code {}",
        code.verification_uri,
        ctx.style.bold(&code.user_code)
    ));
    if !no_browser {
        open_in_browser(&code.verification_uri);
    }
    ctx.say(&ctx.style.dim("waiting for GitHub to confirm…"));
    github_auth::poll_device_flow(&handle).await
}

/// stdin, never an argument: `ps` shows every process's argv to every user on the box.
fn describe_source(source: VarSource) -> &'static str {
    match source {
        VarSource::File => "from the environment file",
        VarSource::Local => "from this machine",
        VarSource::Environment => "from the environment variable",
    }
}

/// The value never travels through `argv`: every process on the box can read
/// another's command line, so a credential passed as an argument is a credential
/// already leaked.
fn read_secret_from_stdin(env: &str, key: &str) -> CoreResult<String> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| CoreError::io("cannot read the value from stdin", e))?;
    let value = raw.trim().to_string();
    if value.is_empty() {
        return Err(CoreError::Secret(format!(
            "no value arrived on stdin for {env}.{key} — pipe it in, e.g. `pbpaste | mandalo env set-secret {env} {key}`"
        )));
    }
    Ok(value)
}

fn read_token_from_stdin() -> CoreResult<String> {
    use std::io::Read;
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| CoreError::io("cannot read the token from stdin", e))?;
    let token = raw.trim().to_string();
    if token.is_empty() {
        return Err(CoreError::Secret(
            "no token arrived on stdin — pipe it in, e.g. `pbpaste | mandalo login --with-token`"
                .to_string(),
        ));
    }
    Ok(token)
}

/// Best effort: the code and the URL are already on screen, so a box without a
/// browser (a server, a container) loses nothing when this does nothing.
fn open_in_browser(url: &str) {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };
    let _ = std::process::Command::new(program)
        .args(args)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn logout(ctx: &Ctx) -> CoreResult<bool> {
    let had_one = github_auth::stored_token()?.is_some();
    github_auth::clear_token()?;
    ctx.say(if had_one {
        "signed out of GitHub"
    } else {
        "there was no GitHub token to forget"
    });
    Ok(true)
}

async fn whoami(ctx: &Ctx, reporter: DataReporter) -> CoreResult<bool> {
    let Some(token) = github_auth::stored_token()? else {
        ctx.say("not signed in to GitHub — run `mandalo login`");
        return Ok(false);
    };
    let user = github_auth::whoami(&token).await?;
    match reporter {
        DataReporter::Json => ctx.say(&to_json(&user)?),
        DataReporter::Pretty => ctx.say(&match &user.name {
            Some(name) => format!("{}  {}", ctx.style.bold(&user.login), ctx.style.dim(name)),
            None => ctx.style.bold(&user.login),
        }),
    }
    Ok(true)
}

/// The token the user already signed in with, or the one CI put in the
/// environment. Registering it here means it can never surface in our output.
fn git_token() -> Option<String> {
    let stored = github_auth::stored_token().ok().flatten();
    stored
        .or_else(|| std::env::var("MANDALO_GIT_TOKEN").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn auth_for(url: &str) -> Auth {
    Auth::for_url(url, git_token().as_deref())
}

fn describe_action(action: SyncAction) -> &'static str {
    match action {
        SyncAction::Nothing => "nothing to do",
        SyncAction::Commit => "commit here (no remote is connected)",
        SyncAction::Push => "push what is already committed",
        SyncAction::CommitAndPush => "commit and push",
        SyncAction::Pull => "pull what changed on the remote",
        SyncAction::BranchAndPush => "commit onto a new branch and push it",
    }
}

fn show_plan(ctx: &Ctx, plan: &SyncPlan) {
    ctx.say(&format!(
        "Sync → {}",
        ctx.style
            .bold(plan.remote.as_deref().unwrap_or("no remote"))
    ));
    ctx.say(&format!(
        "  branch {}",
        plan.branch.as_deref().unwrap_or("(none)")
    ));
    ctx.say(&format!("  {}", describe_action(plan.action)));
    if plan.ahead > 0 || plan.behind > 0 {
        ctx.say(&ctx.style.dim(&format!(
            "  {} ahead, {} behind as of the last fetch",
            plan.ahead, plan.behind
        )));
    }
    ctx.say(&format!(
        "  {} of {} changed",
        plural(plan.included, "file", "files"),
        plan.files.len()
    ));
    for file in &plan.files {
        let label = if file.included {
            format!("{:?}", file.change).to_lowercase()
        } else {
            "left out".to_string()
        };
        let line = format!("    {label:<10} {}", file.path);
        ctx.say(&if file.included {
            line
        } else {
            ctx.style.dim(&line)
        });
    }
    if plan.excluded > 0 {
        ctx.say(&ctx.style.dim(&format!(
            "  {} stay modified here and can go in the next sync",
            plural(plan.excluded, "file", "files")
        )));
    }
    ctx.say(&ctx.style.dim(&format!(
        "  committing as {} <{}>",
        plan.identity.name, plan.identity.email
    )));
    if plan.identity.is_fallback {
        ctx.say(
            &ctx.style
                .warn("  this repository has no user.name or user.email of its own"),
        );
    }
}

fn sync(
    ctx: &Ctx,
    message: &str,
    selection: SyncSelection,
    yes: bool,
    force: bool,
) -> CoreResult<bool> {
    let before = git_sync::status(&ctx.workspace)?;
    if !before.is_repo {
        return Err(CoreError::NotFound(format!(
            "{} is not connected to git yet — run `mandalo git-hygiene` and connect a remote first",
            ctx.workspace.display()
        )));
    }
    let auth = auth_for(before.remote_url.as_deref().unwrap_or_default());
    let plan = git_sync::plan_sync(&ctx.workspace, &selection, None)?;
    show_plan(ctx, &plan);
    show_findings(ctx, &plan.findings, "the commit");
    if plan.blocked && !force {
        return Ok(false);
    }
    if !ctx.agreed(yes, "Sync now?")? {
        ctx.say("nothing was committed or pushed");
        return Ok(false);
    }
    match git_sync::run_sync(
        &ctx.workspace,
        &selection,
        &plan.token,
        message,
        &auth,
        force,
    )? {
        SyncOutcome::NothingToDo => {
            ctx.say("already up to date");
            Ok(true)
        }
        SyncOutcome::Committed { sha, .. } => {
            ctx.say(&format!(
                "committed {}  {}",
                short(&sha),
                ctx.style
                    .dim("no remote is connected, so nothing was pushed")
            ));
            Ok(true)
        }
        SyncOutcome::Pushed { sha, ahead, .. } => {
            ctx.say(&ctx.style.pass(&format!(
                "pushed {} {}",
                ahead,
                if ahead == 1 { "commit" } else { "commits" }
            )));
            ctx.say(&ctx.style.dim(&short(&sha)));
            Ok(true)
        }
        SyncOutcome::Pulled { sha, behind } => {
            ctx.say(&ctx.style.pass(&format!(
                "pulled {} {}",
                behind,
                if behind == 1 { "commit" } else { "commits" }
            )));
            ctx.say(&ctx.style.dim(&short(&sha)));
            Ok(true)
        }
        SyncOutcome::Conflicted { files, .. } => {
            ctx.say(
                &ctx.style
                    .fail("the same lines changed here and on the remote — nothing was pushed"),
            );
            for file in &files {
                ctx.say(&format!("  {file}"));
            }
            ctx.say(
                &ctx.style.dim(
                    "edit those files to agree with the remote, then run `mandalo sync` again",
                ),
            );
            Ok(false)
        }
        SyncOutcome::Rejected { reason } => {
            ctx.say(&ctx.style.fail(&format!("the sync was refused — {reason}")));
            Ok(false)
        }
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

fn dest_for(url: &str, dest: Option<&Path>) -> PathBuf {
    if let Some(dest) = dest {
        return dest.to_path_buf();
    }
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("workspace")
        .trim_end_matches(".git");
    PathBuf::from(if name.is_empty() { "workspace" } else { name })
}

fn dest_for_remote(source: &remote::RemoteSource, dest: Option<&Path>) -> PathBuf {
    if let Some(dest) = dest {
        return dest.to_path_buf();
    }
    match source {
        remote::RemoteSource::Repo { name, subdir, .. } => PathBuf::from(
            subdir
                .as_deref()
                .and_then(|d| d.rsplit('/').next())
                .unwrap_or(name),
        ),
        remote::RemoteSource::Document { url } => PathBuf::from(
            url.rsplit('/')
                .next()
                .unwrap_or("collection")
                .trim_end_matches(".json"),
        ),
    }
}

fn show_review(ctx: &Ctx, review: &remote::RemoteReview) {
    ctx.say(&format!("Open {}", ctx.style.bold(&review.origin.label)));
    if let Some(commit) = &review.origin.commit {
        ctx.say(&ctx.style.dim(&format!("  at commit {commit}")));
    }
    ctx.say(&format!(
        "  {} in {} · {} · {} bytes",
        plural(review.requests, "request", "requests"),
        plural(review.collections, "collection", "collections"),
        plural(review.files, "file", "files"),
        review.bytes
    ));
    ctx.say(&if review.hosts.is_empty() {
        "  contacts no host it names outright".to_string()
    } else {
        format!("  will contact: {}", review.hosts.join(", "))
    });
    for template in &review.templated_hosts {
        ctx.say(&ctx.style.dim(&format!(
            "  and whatever {template} resolves to — you set that"
        )));
    }
    for env in &review.environments {
        ctx.say(&format!(
            "  environment {}: {} declared, {} with a value in the file, {} you would have to supply",
            env.name,
            env.declared.len(),
            env.shared_values,
            env.awaiting_values
        ));
    }
    if review.scripts.is_empty() {
        ctx.say("  carries no scripts");
    } else {
        ctx.say(&ctx.style.warn(&format!(
            "  carries {} — none of them run until you send that request",
            plural(review.scripts.len(), "script", "scripts")
        )));
        for note in &review.scripts {
            ctx.say(&ctx.style.dim(&format!(
                "    {} · {} · {} {}",
                note.collection,
                note.request,
                note.hook,
                plural(note.lines, "line", "lines")
            )));
        }
    }
    for skipped in &review.skipped {
        ctx.say(&ctx.style.dim(&format!("  not taken: {skipped}")));
    }
    show_findings(ctx, &review.findings, "this collection");
}

/// Reads a published collection, shows exactly what it is, and only then writes
/// it — read-only, into a workspace of its own. Nothing in it is executed here:
/// the files are parsed, never run.
async fn open_remote(
    ctx: &Ctx,
    source: &str,
    dest: Option<&Path>,
    yes: bool,
    review_only: bool,
    endpoints: &remote::Endpoints,
) -> CoreResult<bool> {
    let parsed = remote::parse_source(source)?;
    let fetched = match remote::fetch(&parsed, endpoints, ctx.policy().as_ref()).await {
        Ok(fetched) => fetched,
        Err(CoreError::Private(message)) => {
            ctx.say(&ctx.style.fail(&message));
            if let Some(url) = remote::clone_url(&parsed) {
                ctx.say(
                    &ctx.style
                        .dim(&format!("  mandalo login && mandalo clone {url}")),
                );
            }
            return Ok(false);
        }
        Err(other) => return Err(other),
    };
    let review = remote::review(&fetched)?;
    if review_only {
        ctx.say(&to_json(&review)?);
        return Ok(true);
    }
    show_review(ctx, &review);

    if !review.findings.is_empty() {
        ctx.say(&ctx.style.warn(
            "  a credential-looking literal in a stranger's collection is somebody's mistake or somebody's bait — read it before you send anything",
        ));
    }
    let dest = dest_for_remote(&parsed, dest);
    let absolute = if dest.is_absolute() {
        dest.clone()
    } else {
        std::env::current_dir()
            .map_err(|e| CoreError::io("the current directory", e))?
            .join(&dest)
    };
    if !ctx.agreed(
        yes,
        &format!("Open this read-only in {}?", absolute.display()),
    )? {
        ctx.say("nothing was opened");
        return Ok(false);
    }

    let info = remote::adopt(
        &fetched,
        &review.token,
        &workspace::registry_path()?,
        &absolute,
    )?;
    ctx.say(
        &ctx.style
            .pass(&format!("opened {} read-only at {}", info.name, info.path)),
    );
    ctx.say(&ctx.style.dim(
        "it is a copy of somebody else's collection: nothing in it can be edited, and nothing has run. `mandalo save-copy <dir>` makes it yours.",
    ));
    Ok(true)
}

/// Turns the read-only copy into an ordinary workspace the user owns.
fn save_copy(ctx: &Ctx, dest: &Path, name: Option<&str>) -> CoreResult<bool> {
    let absolute = if dest.is_absolute() {
        dest.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| CoreError::io("the current directory", e))?
            .join(dest)
    };
    let name = name
        .map(str::to_string)
        .or_else(|| {
            absolute
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Workspace".to_string());
    let info = remote::save_copy(
        &ctx.workspace,
        &workspace::registry_path()?,
        &absolute,
        &name,
    )?;
    ctx.say(&ctx.style.pass(&format!(
        "saved a copy you own at {} — it is writable and no longer tracks where it came from",
        info.path
    )));
    Ok(true)
}

fn clone(ctx: &Ctx, url: &str, dest: Option<&Path>) -> CoreResult<bool> {
    let dest = dest_for(url, dest);
    git_sync::clone(url, &dest, &auth_for(url))?;
    ctx.say(
        &ctx.style
            .pass(&format!("workspace cloned into {}", dest.display())),
    );
    let findings = scan::scan_workspace(&dest)?;
    for finding in &findings {
        ctx.say(&ctx.style.warn(&format!(
            "the cloned workspace carries a credential-looking literal at {}:{} ({})",
            finding.path.display(),
            finding.line,
            finding.rule
        )));
    }
    ctx.say(
        &ctx.style
            .dim("open it with `mandalo --workspace <path> ls`"),
    );
    Ok(true)
}

fn run_scan(ctx: &Ctx, staged: bool) -> CoreResult<bool> {
    let findings = if staged {
        scan::scan_staged(&ctx.workspace)?
    } else {
        scan::scan_workspace(&ctx.workspace)?
    };
    for finding in &findings {
        ctx.say(&format!(
            "{} {}:{} {}",
            ctx.style.fail("credential"),
            finding.path.display(),
            finding.line,
            ctx.style
                .dim(&format!("{} · {}", finding.rule, finding.excerpt))
        ));
    }
    if findings.is_empty() {
        ctx.say(&ctx.style.pass("no credential-looking literals found"));
        return Ok(true);
    }
    ctx.say(&ctx.style.fail(&format!(
        "{} credential-looking literals found",
        findings.len()
    )));
    Ok(false)
}
