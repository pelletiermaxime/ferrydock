use std::{collections::HashSet, env, time::Duration};

use anyhow::{Context, Result, bail};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults},
    query_parameters::{
        ListContainersOptionsBuilder, LogsOptionsBuilder, RestartContainerOptionsBuilder,
        StopContainerOptionsBuilder,
    },
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::{
    io::AsyncWriteExt,
    process::Command as Process,
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::model::Container;

#[derive(Clone, Debug)]
pub struct Connection {
    pub name: String,
    pub endpoint: String,
}

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
        }
    }
}

pub enum Command {
    Refresh,
    Logs {
        id: String,
        generation: u64,
    },
    CloseLogs,
    Inspect {
        id: String,
        generation: u64,
    },
    Action {
        id: String,
        action: Action,
    },
    Terminal {
        id: String,
        generation: u64,
        shell: Shell,
        columns: u16,
        rows: u16,
    },
    Input {
        generation: u64,
        data: Vec<u8>,
    },
    Resize {
        generation: u64,
        columns: u16,
        rows: u16,
    },
    CloseTerminal,
}

#[derive(Clone, Copy, Debug)]
pub enum Shell {
    Auto,
    Bash,
    Sh,
}

impl Shell {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Bash => "Bash",
            Self::Sh => "sh",
        }
    }
    fn command(self) -> Vec<String> {
        match self {
            Self::Auto => vec![
                "sh",
                "-c",
                "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi",
            ],
            Self::Bash => vec!["bash"],
            Self::Sh => vec!["sh"],
        }
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
}

pub enum Event {
    Connected(Connection),
    Disconnected(String),
    Snapshot(Vec<Container>),
    Log {
        generation: u64,
        data: Vec<u8>,
    },
    LogsEnded {
        generation: u64,
        error: Option<String>,
    },
    Inspection {
        generation: u64,
        result: Result<String, String>,
    },
    ActionStarted {
        id: String,
        action: Action,
    },
    ActionFinished {
        id: String,
        result: Result<(), String>,
    },
    TerminalReady {
        generation: u64,
    },
    TerminalData {
        generation: u64,
        data: Vec<u8>,
    },
    TerminalEnded {
        generation: u64,
        error: Option<String>,
    },
}

pub struct Backend {
    pub commands: mpsc::UnboundedSender<Command>,
    cancel: CancellationToken,
}

impl Backend {
    #[cfg(all(test, feature = "headless-tests"))]
    pub(crate) fn fixture() -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (commands, receiver) = mpsc::unbounded_channel();
        (
            Self {
                commands,
                cancel: CancellationToken::new(),
            },
            receiver,
        )
    }

    pub fn start() -> (Self, mpsc::Receiver<Event>) {
        let (commands, rx) = mpsc::unbounded_channel();
        let (events, output) = mpsc::channel(256);
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        std::thread::Builder::new()
            .name("ferrydock-docker".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("Docker runtime");
                runtime.block_on(run(rx, events, worker_cancel));
            })
            .expect("Docker worker");
        (Self { commands, cancel }, output)
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn context_output(args: &[&str]) -> Result<String> {
    let output = time::timeout(
        Duration::from_secs(8),
        Process::new("docker").args(args).output(),
    )
    .await
    .context("Docker context lookup timed out")??;
    if !output.status.success() {
        bail!(
            "Docker context lookup: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub async fn resolve_connection() -> Result<Connection> {
    if let Ok(context) = env::var("DOCKER_CONTEXT")
        && !context.is_empty()
    {
        let endpoint = context_output(&[
            "context",
            "inspect",
            &context,
            "--format",
            "{{json .Endpoints.docker.Host}}",
        ])
        .await?;
        return Ok(Connection {
            name: context,
            endpoint: serde_json::from_str(&endpoint)?,
        });
    }
    if let Ok(endpoint) = env::var("DOCKER_HOST")
        && !endpoint.is_empty()
    {
        return Ok(Connection {
            name: "DOCKER_HOST".into(),
            endpoint,
        });
    }
    match context_output(&["context", "show"]).await {
        Ok(name) => {
            let endpoint = context_output(&[
                "context",
                "inspect",
                &name,
                "--format",
                "{{json .Endpoints.docker.Host}}",
            ])
            .await?;
            Ok(Connection {
                name,
                endpoint: serde_json::from_str(&endpoint)?,
            })
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            let rootless = env::var("XDG_RUNTIME_DIR")
                .ok()
                .map(|dir| format!("{dir}/docker.sock"));
            let socket = rootless
                .filter(|path| std::path::Path::new(path).exists())
                .unwrap_or_else(|| "/var/run/docker.sock".into());
            Ok(Connection {
                name: "local".into(),
                endpoint: format!("unix://{socket}"),
            })
        }
        Err(error) => Err(error),
    }
}

pub async fn connect(connection: &Connection) -> Result<Docker> {
    let Some(socket) = connection.endpoint.strip_prefix("unix://") else {
        bail!(
            "{} uses {}. This version supports local Unix socket contexts; select a local Docker context.",
            connection.name,
            connection.endpoint
        );
    };
    Ok(time::timeout(
        Duration::from_secs(10),
        Docker::connect_with_unix(socket, 15, API_DEFAULT_VERSION)?.negotiate_version(),
    )
    .await
    .context("Docker connection timed out")??)
}

pub async fn snapshot(docker: &Docker) -> Result<Vec<Container>> {
    Ok(docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default().all(true).build(),
        ))
        .await?
        .into_iter()
        .map(Container::from_summary)
        .collect())
}

pub async fn perform(docker: &Docker, id: &str, action: Action) -> Result<()> {
    match action {
        Action::Start => docker.start_container(id, None).await?,
        Action::Stop => {
            docker
                .stop_container(
                    id,
                    Some(StopContainerOptionsBuilder::default().t(10).build()),
                )
                .await?
        }
        Action::Restart => {
            docker
                .restart_container(
                    id,
                    Some(RestartContainerOptionsBuilder::default().t(10).build()),
                )
                .await?
        }
    }
    Ok(())
}

async fn run(
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    loop {
        let connection = tokio::select! { _ = cancel.cancelled() => return, result = async {
            let connection = resolve_connection().await?;
            let docker = connect(&connection).await?;
            Ok::<_, anyhow::Error>((connection, docker))
        } => result };
        match connection {
            Ok((connection, docker)) => {
                let _ = events.send(Event::Connected(connection)).await;
                let session_cancel = cancel.child_token();
                let result = connected(docker, &mut commands, &events, &session_cancel).await;
                session_cancel.cancel();
                if cancel.is_cancelled() || commands.is_closed() {
                    return;
                }
                let _ = events
                    .send(Event::Disconnected(
                        result
                            .err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "Docker connection closed".into()),
                    ))
                    .await;
            }
            Err(error) => {
                let _ = events.send(Event::Disconnected(error.to_string())).await;
            }
        }
        // Reconnect while remaining responsive to quitting and an explicit refresh.
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = time::sleep(Duration::from_secs(5)) => {},
            command = commands.recv() => if command.is_none() { return; },
        }
    }
}

async fn connected(
    docker: Docker,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    events: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> Result<()> {
    let (changed, mut changes) = watch::channel(0u64);
    let watched_docker = docker.clone();
    let watcher_cancel = cancel.clone();
    tokio::spawn(async move {
        let mut stream = watched_docker.events(None);
        loop {
            tokio::select! {
                _ = watcher_cancel.cancelled() => break,
                event = stream.next() => {
                    if let Some(Ok(_)) = event { changed.send_modify(|value| *value += 1); }
                    else { break; }
                }
            }
        }
    });
    let mut interval = time::interval(Duration::from_secs(10));
    let mut dirty = true;
    let mut logs_cancel = cancel.child_token();
    let mut terminal_cancel = cancel.child_token();
    let mut terminal = None::<(u64, mpsc::UnboundedSender<TerminalCommand>)>;
    let mut busy = HashSet::new();
    let mut actions = FuturesUnordered::new();
    loop {
        if dirty {
            events
                .send(Event::Snapshot(snapshot(&docker).await?))
                .await?;
            dirty = false;
        }
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = interval.tick() => dirty = true,
            change = changes.changed(), if !changes.has_changed().is_err() => { if change.is_ok() { dirty = true; } },
            Some(result) = actions.next(), if !actions.is_empty() => {
                let (id, result) = result?;
                busy.remove(&id);
                let _ = events.send(Event::ActionFinished { id, result }).await;
                dirty = true;
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                match command {
                    Command::Refresh => dirty = true,
                    Command::CloseLogs => logs_cancel.cancel(),
                    Command::CloseTerminal => { terminal_cancel.cancel(); terminal = None; },
                    Command::Logs { id, generation } => {
                        logs_cancel.cancel(); logs_cancel = cancel.child_token();
                        tokio::spawn(stream_logs(docker.clone(), id, generation, events.clone(), logs_cancel.clone()));
                    }
                    Command::Inspect { id, generation } => {
                        let docker = docker.clone(); let events = events.clone();
                        tokio::spawn(async move {
                            let result = docker.inspect_container(&id, None).await.map_err(|e| e.to_string()).and_then(|value| serde_json::to_string_pretty(&value).map_err(|e| e.to_string()));
                            let _ = events.send(Event::Inspection { generation, result }).await;
                        });
                    }
                    Command::Action { id, action } if !busy.contains(&id) => {
                        busy.insert(id.clone());
                        let _ = events.send(Event::ActionStarted { id: id.clone(), action }).await;
                        let docker = docker.clone();
                        actions.push(tokio::spawn(async move {
                            let result = perform(&docker, &id, action).await.map_err(|e| e.to_string());
                            (id, result)
                        }));
                    }
                    Command::Action { .. } => {},
                    Command::Terminal { id, generation, shell, columns, rows } => {
                        terminal_cancel.cancel(); terminal_cancel = cancel.child_token();
                        let (tx, rx) = mpsc::unbounded_channel(); terminal = Some((generation, tx));
                        tokio::spawn(terminal_session(docker.clone(), id, generation, shell, columns, rows, rx, events.clone(), terminal_cancel.clone()));
                    }
                    Command::Input { generation, data } => {
                        if let Some((active, sender)) = &terminal && generation == *active { let _ = sender.send(TerminalCommand::Input(data)); }
                    }
                    Command::Resize { generation, columns, rows } => {
                        if let Some((active, sender)) = &terminal && generation == *active { let _ = sender.send(TerminalCommand::Resize(columns, rows)); }
                    }
                }
            }
        }
    }
}

async fn stream_logs(
    docker: Docker,
    id: String,
    generation: u64,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let options = LogsOptionsBuilder::default()
        .follow(true)
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .tail("500")
        .build();
    let mut stream = docker.logs(&id, Some(options));
    let error = loop {
        tokio::select! {
            _ = cancel.cancelled() => break None,
            frame = stream.next() => match frame {
                Some(Ok(frame)) => { if events.send(Event::Log { generation, data: frame.into_bytes().to_vec() }).await.is_err() { return; } }
                Some(Err(error)) => break Some(error.to_string()),
                None => break None,
            }
        }
    };
    let _ = events.send(Event::LogsEnded { generation, error }).await;
}

enum TerminalCommand {
    Input(Vec<u8>),
    Resize(u16, u16),
}

#[allow(clippy::too_many_arguments)]
async fn terminal_session(
    docker: Docker,
    id: String,
    generation: u64,
    shell: Shell,
    columns: u16,
    rows: u16,
    mut commands: mpsc::UnboundedReceiver<TerminalCommand>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let result = async {
        let exec = docker
            .create_exec(
                &id,
                CreateExecOptions {
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(true),
                    cmd: Some(shell.command()),
                    env: Some(vec![
                        "TERM=xterm-256color".into(),
                        "COLORTERM=truecolor".into(),
                    ]),
                    ..Default::default()
                },
            )
            .await
            .context(
                "Cannot open shell. Try sh if Bash is unavailable; this image may have no shell",
            )?;
        let attached = docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: true,
                    ..Default::default()
                }),
            )
            .await?;
        let StartExecResults::Attached {
            mut output,
            mut input,
        } = attached
        else {
            bail!("Docker did not attach the terminal");
        };
        docker
            .resize_exec(
                &exec.id,
                ResizeExecOptions {
                    height: rows,
                    width: columns,
                },
            )
            .await?;
        let _ = events.send(Event::TerminalReady { generation }).await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => { let _ = input.shutdown().await; break; },
                frame = output.next() => match frame {
                    Some(Ok(frame)) => { events.send(Event::TerminalData { generation, data: frame.into_bytes().to_vec() }).await?; }
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                },
                command = commands.recv() => match command {
                    Some(TerminalCommand::Input(data)) => { input.write_all(&data).await?; input.flush().await?; },
                    Some(TerminalCommand::Resize(columns, rows)) => { docker.resize_exec(&exec.id, ResizeExecOptions { height: rows, width: columns }).await?; },
                    None => { let _ = input.shutdown().await; break; },
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    let error = tokio::select! { _ = cancel.cancelled() => None, result = result => result.err().map(|error| error.to_string()) };
    let _ = events
        .send(Event::TerminalEnded { generation, error })
        .await;
}
