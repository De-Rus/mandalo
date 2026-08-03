use clap::{Parser, Subcommand};
use mandalo_cli::listen;
use mandalo_cli::report::{json_request, render, to_json, DataReporter, JsonSend, Reporter};
use mandalo_cli::style::Style;
use mandalo_core::git_sync::{self, Auth, SyncOutcome};
use mandalo_core::runner::{Runner, StepResult, VarFrame};
use mandalo_core::stream::{Outgoing, StreamKind, StreamSpec, Subscription};
use mandalo_core::{
    bundle, collection, git, github_auth, interpolate, postman, redact, scan, workspace, AllowAll,
    CoreError, CoreResult, EnvVarStore, HostPolicy, Redactor, SecretStore, StrictPolicy,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "mandalo",
    version,
    about = "Mándalo — run API collections from a terminal or from CI",
    disable_help_subcommand = true
)]
struct Cli {
    /// Workspace directory or registered workspace id (defaults to the active workspace)
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
    Send {
        /// Collection slug
        collection: String,
        /// Request path inside the collection, e.g. auth/login.toml
        request_path: String,
        #[arg(long, value_name = "NAME")]
        env: Option<String>,
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
    /// Open a realtime stream and print every event until it closes
    Listen {
        /// ws:// wss:// http:// https:// mqtt:// mqtts://
        url: String,
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
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// Print the collection tree
    Ls {
        #[arg(long, value_enum, default_value_t = DataReporter::Pretty)]
        reporter: DataReporter,
    },
    /// Import a Postman collection or a Mándalo bundle
    Import { file: PathBuf },
    /// Export the workspace as a Mándalo bundle
    Export { file: PathBuf },
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
        /// Commit even though the credential scanner found something
        #[arg(long)]
        force: bool,
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
    /// Write one non-secret variable into an environment
    Set {
        name: String,
        key: String,
        value: String,
    },
}

struct Ctx {
    workspace: PathBuf,
    style: Style,
    redactor: &'static Redactor,
    strict: bool,
}

impl Ctx {
    fn say(&self, text: &str) {
        println!("{}", self.redactor.scrub(text));
    }

    fn policy(&self) -> Box<dyn HostPolicy> {
        if self.strict {
            Box::new(StrictPolicy::new())
        } else {
            Box::new(AllowAll)
        }
    }

    fn runner(&self) -> Runner<EnvVarStore, Box<dyn HostPolicy>> {
        Runner::new(EnvVarStore, self.policy()).with_workspace(self.workspace.clone())
    }
}

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
        return workspace::resolve_workspace(&workspace::registry_path()?, raw);
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

/// Signing in is what a brand-new user does *before* there is anything to sign in
/// for, so the GitHub commands must not die on "there is no workspace yet".
fn needs_workspace(command: &Command) -> bool {
    !matches!(
        command,
        Command::Login { .. }
            | Command::Logout
            | Command::Whoami { .. }
            | Command::Clone { .. }
            | Command::Listen { env: None, .. }
    )
}

fn main() {
    let cli = Cli::parse();
    let style = Style::for_stdout();
    let workspace = match resolve_workspace(cli.workspace.as_deref()) {
        Ok(path) => path,
        Err(_) if !needs_workspace(&cli.command) => PathBuf::new(),
        Err(e) => {
            eprintln!("{}", style.fail(&format!("{} [{}]", e, e.code())));
            std::process::exit(1);
        }
    };
    let ctx = Ctx {
        workspace,
        style,
        redactor: redact::global(),
        strict: cli.strict_network,
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
        Command::Env { command } => env_command(ctx, command),
        Command::Ls { reporter } => list_tree(ctx, reporter),
        Command::Import { file } => import(ctx, &file),
        Command::Export { file } => export(ctx, &file),
        Command::Scan { staged } => run_scan(ctx, staged),
        Command::GitHygiene { install_hook } => git_hygiene(ctx, install_hook),
        Command::Sync { message, force } => sync(ctx, &message, force),
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

async fn listen_to(ctx: &Ctx, request: ListenRequest) -> CoreResult<bool> {
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
    let frame = load_frame(ctx, request.env.as_deref())?;
    refuse_secrets(&frame, &request.url, &spec.headers)?;
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
        StreamKind::Mqtt => {
            let topic = request.topics.first().cloned().ok_or_else(|| {
                CoreError::Stream(
                    "an mqtt publish needs a topic — pass --topic".to_string(),
                )
            });
            request
                .send
                .iter()
                .map(|text| {
                    Ok(Outgoing::Publish {
                        topic: topic.clone()?,
                        payload: text.clone(),
                        qos: request.qos,
                        retain: false,
                    })
                })
                .collect::<CoreResult<Vec<_>>>()?
        }
        StreamKind::Sse if !request.send.is_empty() => {
            return Err(CoreError::Stream(
                "server-sent events only travel from the server to the client — there is nothing to send"
                    .to_string(),
            ))
        }
        _ => request.send.iter().map(Outgoing::text).collect(),
    };

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
        .runner()
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
    let mut step = match ctx.runner().run_request(&request, &mut vars).await {
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

fn load_frame(ctx: &Ctx, env: Option<&str>) -> CoreResult<VarFrame> {
    match env {
        Some(name) => {
            let listed = workspace::list_environments(&ctx.workspace)?;
            let found = listed
                .items
                .into_iter()
                .find(|e| e.name == name)
                .ok_or_else(|| CoreError::NotFound(format!("unknown environment: {name}")))?;
            Ok(VarFrame::from_environment(&found))
        }
        None => Ok(VarFrame::default()),
    }
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
                let secrets = env.vars.values().filter(|v| v.is_secret()).count();
                let detail = if secrets == 0 {
                    format!("{} variables", env.vars.len())
                } else {
                    format!("{} variables · {secrets} secret", env.vars.len())
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
            for (key, def) in &doc.vars {
                match def {
                    workspace::VarDef::Plain { value } => ctx.say(&format!("{key}={value}")),
                    workspace::VarDef::Secret { hosts } => {
                        let bound = if hosts.is_empty() {
                            "not bound to a host".to_string()
                        } else {
                            format!("for {}", hosts.join(", "))
                        };
                        let held = EnvVarStore.get(&name, key)?.is_some();
                        ctx.say(&format!(
                            "{key}={}",
                            ctx.style.dim(&format!(
                                "secret · {bound} · {}",
                                if held {
                                    "value present"
                                } else {
                                    "no value in this shell"
                                }
                            ))
                        ));
                    }
                }
            }
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
            workspace::save_environment(&ctx.workspace, &env)?;
            ctx.say(&format!("{name}.{key} saved"));
            Ok(true)
        }
    }
}

fn list_tree(ctx: &Ctx, reporter: DataReporter) -> CoreResult<bool> {
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

fn import(ctx: &Ctx, file: &Path) -> CoreResult<bool> {
    let json = std::fs::read_to_string(file).map_err(|e| CoreError::io(file.display(), e))?;
    let report = if json.contains("\"mandaloBundle\"") {
        bundle::import(&ctx.workspace, &json)?
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

fn export(ctx: &Ctx, file: &Path) -> CoreResult<bool> {
    let exported = bundle::export(&ctx.workspace)?;
    std::fs::write(file, &exported.json).map_err(|e| CoreError::io(file.display(), e))?;
    ctx.say(&format!("workspace exported to {}", file.display()));
    for finding in &exported.findings {
        ctx.say(&ctx.style.warn(&format!(
            "the bundle carries a credential-looking literal on line {} ({} · {})",
            finding.line, finding.rule, finding.excerpt
        )));
    }
    Ok(exported.findings.is_empty())
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

fn sync(ctx: &Ctx, message: &str, force: bool) -> CoreResult<bool> {
    let before = git_sync::status(&ctx.workspace)?;
    if !before.is_repo {
        return Err(CoreError::NotFound(format!(
            "{} is not connected to git yet — run `mandalo git-hygiene` and connect a remote first",
            ctx.workspace.display()
        )));
    }
    let auth = auth_for(before.remote_url.as_deref().unwrap_or_default());
    if let Some(identity) = before.identity.as_ref().filter(|i| i.is_fallback) {
        ctx.say(&ctx.style.warn(&format!(
            "this repository has no user.name or user.email — committing as {} <{}>",
            identity.name, identity.email
        )));
    }
    match git_sync::sync(&ctx.workspace, message, &auth, force)? {
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
        SyncOutcome::Conflicted { files } => {
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
        scan::scan_files(&scan::staged_files(&ctx.workspace)?)?
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
