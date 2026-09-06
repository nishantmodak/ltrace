use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use ltrace_local::{
    local::{self, Client},
    model::Expectation,
};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::process::Command;

#[derive(Parser)]
#[command(
    name = "ltrace-dev",
    version,
    about = "Local OpenTelemetry evidence for coding agents. Not the Linux ltrace syscall utility."
)]
struct Args {
    #[arg(long, global = true, env = "LTRACE_HOME")]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Start the loopback receiver. The desktop app can also start it.
    Serve {
        #[arg(long, default_value_t = 4318)]
        port: u16,
    },
    /// Check the receiver identity and connection.
    Doctor,
    /// Create a debugging session. Expectations are a JSON array (see example).
    Session {
        #[arg(long)]
        title: String,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        expectations: Option<PathBuf>,
    },
    /// List recent sessions.
    Sessions,
    /// Read a session or run summary as bounded JSON.
    Show {
        #[arg(value_parser=["session","run"])]
        kind: String,
        id: String,
    },
    /// List requests without loading raw attributes, with optional operation/service search.
    Traces {
        run: String,
        #[arg(long)]
        search: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Read raw evidence with pagination.
    Spans {
        run: String,
        #[arg(long)]
        trace: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Inspect a span and recorded child coverage.
    Span {
        run: String,
        trace: String,
        span: String,
    },
    /// Publish a concise observation to the desktop session.
    Note {
        session: String,
        #[arg(long)]
        run: Option<String>,
        #[arg(long)]
        body: String,
    },
    /// Run a real test with isolated local OTel export. Child output goes to stderr.
    Capture {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        expectations: Option<PathBuf>,
        #[arg(long, default_value = "Test run")]
        label: String,
        /// Safe description stored in history; arguments and output are never stored.
        #[arg(long)]
        command_label: Option<String>,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
        /// Bounded grace after child exits. The SDK must flush before exit.
        #[arg(long, default_value_t = 500)]
        settle_ms: u64,
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    match run(Args::parse()).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("ltrace: {e:#}");
            std::process::exit(2);
        }
    }
}
fn print(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn run(args: Args) -> Result<i32> {
    let dir = args.home.unwrap_or(local::data_dir()?);
    if let Action::Serve { port } = args.command {
        let server = local::start(&dir, port).await?;
        eprintln!(
            "ltrace listening at {} (data: {})",
            server.info.url,
            dir.display()
        );
        tokio::signal::ctrl_c().await?;
        drop(server);
        return Ok(0);
    }
    let client = Client::connect(&dir)?;
    client.health().await?;
    let value = match args.command {
        Action::Doctor => {
            json!({"receiver":client.health().await?,"url":client.info.url,"data_dir":dir})
        }
        Action::Session {
            title,
            project,
            expectations,
        } => {
            let project = project.unwrap_or(std::env::current_dir()?).canonicalize()?;
            let expectations: Vec<Expectation> = if let Some(path) = expectations {
                serde_json::from_slice(&std::fs::read(path)?)?
            } else {
                vec![]
            };
            client
                .write(
                    "sessions",
                    json!({"title":title,"project":project,"expectations":expectations}),
                )
                .await?
        }
        Action::Sessions => client.read("sessions").await?,
        Action::Show { kind, id } => {
            valid_id(&id)?;
            client.read(&format!("{kind}s/{id}")).await?
        }
        Action::Traces {
            run,
            search,
            offset,
            limit,
        } => {
            valid_id(&run)?;
            let mut url = reqwest::Url::parse("http://127.0.0.1/")?;
            url.query_pairs_mut()
                .append_pair("q", search.as_deref().unwrap_or(""))
                .append_pair("offset", &offset.to_string())
                .append_pair("limit", &limit.to_string());
            client
                .read(&format!("runs/{run}/traces?{}", url.query().unwrap_or("")))
                .await?
        }
        Action::Spans {
            run,
            trace,
            offset,
            limit,
        } => {
            valid_id(&run)?;
            if let Some(t) = &trace {
                ensure!(
                    t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid trace ID"
                );
            }
            client
                .read(&format!(
                    "runs/{run}/spans?offset={offset}&limit={limit}{}",
                    trace.map(|s| format!("&trace_id={s}")).unwrap_or_default()
                ))
                .await?
        }
        Action::Span { run, trace, span } => {
            valid_id(&run)?;
            ensure!(
                trace.len() == 32
                    && span.len() == 16
                    && trace
                        .bytes()
                        .chain(span.bytes())
                        .all(|b| b.is_ascii_hexdigit()),
                "invalid span reference"
            );
            client
                .read(&format!("runs/{run}/spans/{trace}/{span}"))
                .await?
        }
        Action::Note { session, run, body } => {
            valid_id(&session)?;
            client
                .write(
                    &format!("sessions/{session}/notes"),
                    json!({"run_id":run,"body":body}),
                )
                .await?
        }
        Action::Capture {
            session,
            expectations,
            label,
            command_label,
            timeout_seconds,
            settle_ms,
            command,
        } => {
            return capture(
                &client,
                (session, expectations),
                label,
                command_label,
                timeout_seconds,
                settle_ms,
                command,
            )
            .await;
        }
        Action::Serve { .. } => unreachable!(),
    };
    print(&value)?;
    Ok(0)
}
fn valid_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)?;
    Ok(())
}

async fn capture(
    client: &Client,
    setup: (Option<String>, Option<PathBuf>),
    label: String,
    command_label: Option<String>,
    timeout: u64,
    settle: u64,
    argv: Vec<String>,
) -> Result<i32> {
    let (session, expectation_file) = setup;
    ensure!(
        (1..=3600).contains(&timeout) && settle <= 10_000,
        "timeout must be 1–3600 seconds; settle at most 10000 ms"
    );
    let cwd = std::env::current_dir()?.canonicalize()?;
    let session = if let Some(session) = session {
        valid_id(&session)?;
        let data = client.read(&format!("sessions/{session}")).await?;
        let project = PathBuf::from(
            data["session"]["project"]
                .as_str()
                .context("missing project")?,
        )
        .canonicalize()?;
        ensure!(
            cwd.starts_with(project),
            "run the capture command inside the selected project"
        );
        session
    } else {
        let root = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .await;
        let project = match root {
            Ok(output) if output.status.success() => {
                PathBuf::from(String::from_utf8(output.stdout)?.trim()).canonicalize()?
            }
            _ => cwd.clone(),
        };
        let project = client
            .write("projects/ensure", json!({"project":project}))
            .await?;
        project["id"]
            .as_str()
            .context("project ID missing")?
            .to_owned()
    };
    let expectations: Vec<Expectation> = match expectation_file {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)?,
        None => vec![],
    };
    let revision = revision().await;
    let description = command_label.unwrap_or_else(|| {
        format!(
            "{} (arguments omitted)",
            PathBuf::from(&argv[0])
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    });
    let started = client
        .write(
            &format!("sessions/{session}/runs"),
            json!({"label":label,"command":description,"revision":revision,"expectations":expectations}),
        )
        .await?;
    let id = started["run"]["id"].as_str().context("run ID missing")?;
    let token = started["export_token"]
        .as_str()
        .context("export credential missing")?;
    eprintln!("ltrace run {id}");
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            format!("{}/v1/traces", client.info.url),
        )
        .env("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "http/protobuf")
        .env(
            "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
            format!("x-ltrace-run-token={token}"),
        )
        .env("OTEL_TRACES_SAMPLER", "always_on")
        .env("OTEL_BSP_SCHEDULE_DELAY", "100")
        .env("LTRACE_RUN_ID", id);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let spawned = command.spawn();
    let (exit, mut issue) = match spawned {
        Err(_) => (
            None,
            Some(
                "Test command could not be started; check executable and working directory."
                    .to_string(),
            ),
        ),
        Ok(mut child) => {
            let mut stdout = child.stdout.take().context("missing child stdout")?;
            let mut stderr = child.stderr.take().context("missing child stderr")?;
            let out = tokio::spawn(async move {
                tokio::io::copy(&mut stdout, &mut tokio::io::stderr()).await
            });
            let err = tokio::spawn(async move {
                tokio::io::copy(&mut stderr, &mut tokio::io::stderr()).await
            });
            let pid = child.id();
            let outcome = tokio::select! {
                result = child.wait() => result.map(|s| (s.code(),None)),
                _ = tokio::time::sleep(Duration::from_secs(timeout)) => Ok((None,Some("Test command timed out; capture is incomplete.".to_string()))),
                _ = tokio::signal::ctrl_c() => Ok((None,Some("Test command interrupted; capture is incomplete.".to_string()))),
            };
            let (code, mut issue) = outcome.context("waiting for test command")?;
            if issue.is_some() {
                #[cfg(unix)]
                if let Some(pid) = pid {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            // A background descendant must not keep the capture hanging forever.
            let mut out = out;
            let mut err = err;
            if tokio::time::timeout(Duration::from_secs(1), async {
                let _ = (&mut out).await;
                let _ = (&mut err).await;
            })
            .await
            .is_err()
            {
                out.abort();
                err.abort();
                issue =
                    Some("A descendant kept test output open; capture may be incomplete.".into());
            }
            (code, issue)
        }
    };
    if exit.is_none() && issue.is_none() {
        issue = Some("Test terminated without an exit code.".into());
    }
    tokio::time::sleep(Duration::from_millis(settle)).await;
    let finished = client
        .write(
            &format!("runs/{id}/finish"),
            json!({"exit_code":exit,"issue":issue}),
        )
        .await;
    if let Err(e) = finished {
        eprintln!("Could not finalize capture: {e}. Test exit: {exit:?}");
        return Ok(exit.filter(|c| *c != 0).unwrap_or(2));
    }
    match client.read(&format!("runs/{id}")).await {
        Ok(report) => print(&report)?,
        Err(e) => {
            eprintln!("Could not read capture: {e}");
            return Ok(exit.filter(|c| *c != 0).unwrap_or(2));
        }
    }
    // CLI exit means the test's result. Verification is explicitly separate JSON.
    Ok(exit.unwrap_or(2))
}

async fn revision() -> String {
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .await;
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .await;
    match (head, status) {
        (Ok(h), Ok(s)) if h.status.success() && s.status.success() => format!(
            "{}{}",
            String::from_utf8_lossy(&h.stdout).trim(),
            if s.stdout.is_empty() { "" } else { " dirty" }
        ),
        _ => "unavailable".into(),
    }
}
