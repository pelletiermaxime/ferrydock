use std::{
    collections::{HashMap, HashSet},
    env,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    exec::{ResizeExecOptions, StartExecOptions, StartExecResults},
    query_parameters::{
        ListContainersOptionsBuilder, LogsOptionsBuilder, RestartContainerOptionsBuilder,
        StopContainerOptionsBuilder,
    },
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::{compose, model::Container};

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
    Images {
        generation: u64,
    },
    CloseImages,
    ImageDetails {
        id: String,
        generation: u64,
    },
    CloseImageDetails,
    AnalyzeImage {
        id: String,
        generation: u64,
    },
    CloseImageAnalysis,
    Volumes {
        generation: u64,
    },
    CloseVolumes,
    Networks {
        generation: u64,
    },
    CloseNetworks,
    NetworkDetails {
        id: String,
        generation: u64,
    },
    CloseNetworkDetails,
    GatherVolumeSizes {
        request: u64,
    },
    PruneVolumes {
        mode: crate::volumes::PruneMode,
        operation: u64,
    },
    RemoveVolume {
        name: String,
        operation: u64,
    },
    RemoveVolumes {
        names: Vec<String>,
        operation: u64,
    },
    RemoveImage {
        id: String,
        operation: u64,
    },
    RemoveImages {
        ids: Vec<String>,
        operation: u64,
    },
    PruneImages {
        mode: crate::images::PruneMode,
        operation: u64,
    },
    ConfigureCompose {
        config: compose::Config,
    },
    Compose {
        project: String,
        operation: u64,
        action: compose::Action,
    },
    Logs {
        id: String,
        generation: u64,
    },
    CloseLogs,
    ProjectLogs {
        project: String,
        generation: u64,
    },
    CloseProjectLogs,
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
    FavoriteTerminal {
        id: String,
        generation: u64,
        endpoint: String,
        target: crate::favorites::Target,
        command: String,
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
    Images {
        generation: u64,
        result: Result<Vec<crate::images::Image>, String>,
    },
    ImageHistory {
        id: String,
        generation: u64,
        result: Result<Vec<bollard::models::ImageHistoryResponseItem>, String>,
    },
    ImageAnalysis {
        id: String,
        generation: u64,
        result: Result<std::sync::Arc<crate::image_analysis::Report>, String>,
    },
    ImageInspection {
        id: String,
        generation: u64,
        result: Result<String, String>,
    },
    ImageRemoved {
        id: String,
        operation: u64,
        result: Result<(), String>,
    },
    ImagesRemoved {
        operation: u64,
        results: Vec<crate::images::RemovalResult>,
    },
    ImagesPruned {
        operation: u64,
        result: Result<Option<u64>, String>,
    },
    Volumes {
        generation: u64,
        result: Result<Vec<crate::volumes::Volume>, String>,
    },
    Networks {
        generation: u64,
        result: Result<Vec<crate::networks::Network>, String>,
    },
    NetworkDetails {
        id: String,
        generation: u64,
        result: Result<crate::networks::Network, String>,
    },
    VolumeRemoved {
        name: String,
        operation: u64,
        result: Result<(), String>,
    },
    VolumesRemoved {
        operation: u64,
        results: Vec<crate::volumes::RemovalResult>,
    },
    VolumeSizes {
        request: u64,
        result: Result<Vec<crate::volumes::VolumeSize>, String>,
    },
    VolumesPruned {
        operation: u64,
        result: Result<crate::volumes::PruneResult, String>,
    },
    MetricsSession(u64),
    Metrics {
        session: u64,
        id: String,
        result: Result<crate::metrics::Sample, String>,
    },
    ComposeProjects(Vec<compose::Project>),
    ProjectLog {
        generation: u64,
        line: crate::project_logs::Line,
    },
    ProjectLogBatch {
        generation: u64,
        lines: Vec<crate::project_logs::Line>,
    },
    ProjectLogsStatus {
        generation: u64,
        status: String,
    },
    ComposeSettingsError(String),
    ComposeConfigured {
        project: String,
        result: Result<compose::Config, String>,
    },
    ComposeOutput {
        project: String,
        operation: u64,
        data: Vec<u8>,
    },
    ComposeFinished {
        project: String,
        operation: u64,
        result: Result<(), String>,
    },
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
        crate::docker_cli::command()
            .args(args)
            .kill_on_drop(true)
            .output(),
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
                let _ = events.send(Event::Connected(connection.clone())).await;
                let session_cancel = cancel.child_token();
                let result =
                    connected(docker, connection, &mut commands, &events, &session_cancel).await;
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
    connection: Connection,
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
    let (image_requests, image_receiver) = watch::channel(None);
    tokio::spawn(crate::images::follow(
        docker.clone(),
        image_receiver,
        events.clone(),
        cancel.child_token(),
    ));
    let mut image_jobs = FuturesUnordered::new();
    let (image_detail_requests, image_detail_receiver) = watch::channel(None);
    tokio::spawn(crate::images::follow_details(
        docker.clone(),
        image_detail_receiver,
        events.clone(),
        cancel.child_token(),
    ));
    let (analysis_requests, analysis_receiver) = watch::channel(None);
    tokio::spawn(crate::image_analysis::follow(
        docker.clone(),
        analysis_receiver,
        events.clone(),
        cancel.child_token(),
    ));
    let mut image_busy = HashSet::new();
    let mut prune_jobs = FuturesUnordered::new();
    let mut pruning = false;
    let (volume_requests, volume_receiver) = watch::channel(None);
    tokio::spawn(crate::volumes::follow(
        docker.clone(),
        volume_receiver,
        events.clone(),
        cancel.child_token(),
    ));
    let mut volume_jobs = FuturesUnordered::new();
    let mut volume_busy = HashSet::new();
    let mut volume_size_jobs = FuturesUnordered::new();
    let mut volume_prune_jobs = FuturesUnordered::new();
    let mut logs_cancel = cancel.child_token();
    let mut project_logs_cancel = cancel.child_token();
    let (project_snapshots, snapshots) = watch::channel(Vec::new());
    static METRICS_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let metrics_session = METRICS_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    events.send(Event::MetricsSession(metrics_session)).await?;
    tokio::spawn(crate::metrics::follow(
        docker.clone(),
        metrics_session,
        snapshots.clone(),
        events.clone(),
        cancel.child_token(),
    ));
    let (network_requests, network_receiver) = watch::channel(None);
    tokio::spawn(crate::networks::follow(
        docker.clone(),
        network_receiver,
        events.clone(),
        cancel.child_token(),
    ));
    let (network_detail_requests, network_detail_receiver) = watch::channel(None);
    tokio::spawn(crate::networks::follow(
        docker.clone(),
        network_detail_receiver,
        events.clone(),
        cancel.child_token(),
    ));
    let mut terminal_cancel = cancel.child_token();
    let mut terminal = None::<(u64, mpsc::UnboundedSender<TerminalCommand>)>;
    let mut busy = HashSet::new();
    let mut actions = FuturesUnordered::new();
    let mut compose_jobs = FuturesUnordered::new();
    let mut compose_busy = HashSet::new();
    let mut container_projects = std::collections::HashMap::new();
    let mut project_configs = std::collections::HashMap::new();
    let mut settings =
        match compose::Settings::path().and_then(|path| compose::Settings::load(&path)) {
            Ok(settings) => settings,
            Err(error) => {
                events
                    .send(Event::ComposeSettingsError(format!("{error:#}")))
                    .await?;
                compose::Settings::default()
            }
        };
    loop {
        if dirty {
            let containers = snapshot(&docker).await?;
            project_snapshots.send_replace(containers.clone());
            container_projects = containers
                .iter()
                .map(|container| (container.id.clone(), container.project.clone()))
                .collect();
            let mut projects = settings.projects(&connection.endpoint, &containers);
            let projects = tokio::task::spawn_blocking(move || {
                for project in &mut projects {
                    project.discover_local_source();
                }
                projects
            })
            .await?;
            project_configs = projects
                .iter()
                .filter_map(|project| {
                    project
                        .config
                        .clone()
                        .map(|config| (project.name.clone(), config))
                })
                .collect();
            events.send(Event::ComposeProjects(projects)).await?;
            events.send(Event::Snapshot(containers)).await?;
            if image_requests.borrow().is_some() {
                image_requests.send_modify(|_| {});
            }
            if volume_requests.borrow().is_some() {
                volume_requests.send_modify(|_| {});
            }
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
            Some(result) = compose_jobs.next(), if !compose_jobs.is_empty() => {
                let (project, operation, action, mut result): (String, u64, compose::Action, Result<(), String>) = result?;
                if result.is_ok() && action.is_cleanup() && settings.config(&connection.endpoint, &project).is_some() {
                    match compose::Settings::path().and_then(|path| compose::Settings::forget(&path, &connection.endpoint, &project)) {
                        Ok(updated) => settings = updated,
                        Err(error) => result = Err(format!("Docker cleanup completed, but the saved association could not be removed: {error:#}")),
                    }
                }
                compose_busy.remove(&project);
                let _ = events.send(Event::ComposeFinished { project, operation, result }).await;
                dirty = true;
            }
            Some(result) = image_jobs.next(), if !image_jobs.is_empty() => {
                let (operation, mut results, bulk): (u64, Vec<crate::images::RemovalResult>, bool) = result?;
                for result in &results { image_busy.remove(&result.id); }
                if bulk {
                    events.send(Event::ImagesRemoved { operation, results }).await?;
                } else if let Some(crate::images::RemovalResult { id, result }) = results.pop() {
                    events.send(Event::ImageRemoved { id, operation, result }).await?;
                }
                dirty = true;
            }
            Some(result) = volume_jobs.next(), if !volume_jobs.is_empty() => {
                let (operation, mut results, bulk): (u64, Vec<crate::volumes::RemovalResult>, bool) = result?;
                for result in &results { volume_busy.remove(&result.name); }
                if bulk { events.send(Event::VolumesRemoved { operation, results }).await?; }
                else if let Some(crate::volumes::RemovalResult { name, result }) = results.pop() {
                    events.send(Event::VolumeRemoved { name, operation, result }).await?;
                }
                dirty = true;
            }
            Some(result) = volume_size_jobs.next(), if !volume_size_jobs.is_empty() => {
                let (request, result) = result?;
                events.send(Event::VolumeSizes { request, result }).await?;
            }
            Some(result) = volume_prune_jobs.next(), if !volume_prune_jobs.is_empty() => {
                let (operation, result) = result?;
                pruning = false;
                events.send(Event::VolumesPruned { operation, result }).await?;
                dirty = true;
            }
            Some(result) = prune_jobs.next(), if !prune_jobs.is_empty() => {
                let (operation, result) = result?;
                pruning = false;
                events.send(Event::ImagesPruned { operation, result }).await?;
                dirty = true;
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                match command {
                    Command::Refresh => dirty = true,
                    Command::Images { generation } => { image_requests.send_replace(Some(generation)); },
                    Command::CloseImages => { image_requests.send_replace(None); },
                    Command::ImageDetails { id, generation } => { image_detail_requests.send_replace(Some((id, generation))); },
                    Command::CloseImageDetails => { image_detail_requests.send_replace(None); },
                    Command::AnalyzeImage { id, generation } => { analysis_requests.send_replace(Some((id,generation))); },
                    Command::CloseImageAnalysis => { analysis_requests.send_replace(None); },
                    Command::Volumes { generation } => { volume_requests.send_replace(Some(generation)); },
                    Command::CloseVolumes => { volume_requests.send_replace(None); },
                    Command::Networks { generation } => { network_requests.send_replace(Some((None,generation))); },
                    Command::NetworkDetails { id, generation } => { network_detail_requests.send_replace(Some((Some(id),generation))); },
                    Command::CloseNetworks => { network_requests.send_replace(None); network_detail_requests.send_replace(None); },
                    Command::CloseNetworkDetails => { network_detail_requests.send_replace(None); },
                    Command::PruneVolumes { mode, operation } => {
                        if pruning || !image_busy.is_empty() || !volume_busy.is_empty() || !compose_busy.is_empty() || !busy.is_empty() {
                            events.send(Event::VolumesPruned { operation, result: Err("Wait for the current Docker operation before pruning volumes".into()) }).await?;
                            continue;
                        }
                        pruning = true;
                        let docker = docker.clone();
                        let cancellation = cancel.child_token();
                        volume_prune_jobs.push(tokio::spawn(async move {
                            let result = tokio::select! {
                                _ = cancellation.cancelled() => Err("Docker disconnected; refresh to check volume state".into()),
                                result = crate::volumes::prune(&docker, mode) => result.map_err(|error| format!("{error:#}")),
                            };
                            (operation, result)
                        }));
                    }
                    Command::GatherVolumeSizes { request } => {
                        if !volume_size_jobs.is_empty() {
                            events.send(Event::VolumeSizes { request, result: Err("Volume sizes are already being gathered".into()) }).await?;
                            continue;
                        }
                        let endpoint = connection.endpoint.clone();
                        let version = docker.client_version();
                        let cancellation = cancel.child_token();
                        volume_size_jobs.push(tokio::spawn(async move {
                            let result = tokio::select! {
                                _ = cancellation.cancelled() => Err("Docker disconnected while gathering volume sizes".into()),
                                result = crate::volumes::gather_sizes(&endpoint, &version) => result.map_err(|error| format!("{error:#}")),
                            };
                            (request, result)
                        }));
                    }
                    Command::RemoveVolume { name, operation } => {
                        if volume_busy.contains(&name) { continue; }
                        if pruning || !volume_busy.is_empty() || !image_busy.is_empty() || !compose_busy.is_empty() || !busy.is_empty() {
                            events.send(Event::VolumeRemoved { name, operation, result: Err("Wait for the current Docker operation before removing a volume".into()) }).await?;
                            continue;
                        }
                        volume_busy.insert(name.clone());
                        let docker = docker.clone();
                        let cancellation = cancel.child_token();
                        volume_jobs.push(tokio::spawn(async move {
                            let result = tokio::select! {
                                _ = cancellation.cancelled() => Err("Docker disconnected; refresh to check volume state".into()),
                                result = crate::volumes::remove(&docker, &name) => result.map_err(|error| format!("{error:#}")),
                            };
                            (operation, vec![crate::volumes::RemovalResult { name, result }], false)
                        }));
                    },
                    Command::RemoveVolumes { names, operation } => {
                        if names.is_empty() { continue; }
                        if pruning || !volume_busy.is_empty() || !image_busy.is_empty() || !compose_busy.is_empty() || !busy.is_empty() {
                            let results = names.into_iter().map(|name| crate::volumes::RemovalResult { name, result: Err("Wait for the current Docker operation before removing volumes".into()) }).collect();
                            events.send(Event::VolumesRemoved { operation, results }).await?;
                            continue;
                        }
                        volume_busy.extend(names.iter().cloned());
                        let docker = docker.clone();
                        let cancellation = cancel.child_token();
                        volume_jobs.push(tokio::spawn(async move {
                            let results = tokio::select! {
                                _ = cancellation.cancelled() => names.iter().map(|name| crate::volumes::RemovalResult { name: name.clone(), result: Err("Docker disconnected; refresh to check volume state".into()) }).collect(),
                                results = crate::volumes::remove_many(&docker, names.clone()) => results,
                            };
                            (operation, results, true)
                        }));
                    },
                    Command::RemoveImage { id, operation } => {
                        if image_busy.contains(&id) {
                            continue;
                        }
                        if pruning || !image_busy.is_empty() || !volume_busy.is_empty() || !compose_busy.is_empty() {
                            events.send(Event::ImageRemoved { id, operation, result: Err("Wait for the current image or Compose operation before removing an image".into()) }).await?;
                            continue;
                        }
                        image_busy.insert(id.clone());
                        let docker = docker.clone();
                        let cancellation = cancel.child_token();
                        image_jobs.push(tokio::spawn(async move {
                            let result = tokio::select! {
                                _ = cancellation.cancelled() => Err("Docker disconnected; refresh to check image state".into()),
                                result = crate::images::remove(&docker, &id) => result.map_err(|error| format!("{error:#}")),
                            };
                            (operation, vec![crate::images::RemovalResult { id, result }], false)
                        }));
                    }
                    Command::RemoveImages { ids, operation } => {
                        if ids.is_empty() { continue; }
                        if pruning || !image_busy.is_empty() || !volume_busy.is_empty() || !compose_busy.is_empty() {
                            let results = ids.into_iter().map(|id| crate::images::RemovalResult { id, result: Err("Wait for the current image or Compose operation before removing images".into()) }).collect();
                            events.send(Event::ImagesRemoved { operation, results }).await?;
                            continue;
                        }
                        image_busy.extend(ids.iter().cloned());
                        let docker = docker.clone();
                        let cancellation = cancel.child_token();
                        image_jobs.push(tokio::spawn(async move {
                            let results = tokio::select! {
                                _ = cancellation.cancelled() => ids.iter().map(|id| crate::images::RemovalResult { id: id.clone(), result: Err("Docker disconnected; refresh to check image state".into()) }).collect(),
                                results = crate::images::remove_many(&docker, ids.clone()) => results,
                            };
                            (operation, results, true)
                        }));
                    }
                    Command::ProjectLogs { project, generation } => {
                        project_logs_cancel.cancel(); project_logs_cancel = cancel.child_token();
                        tokio::spawn(crate::project_logs::follow(docker.clone(), project, generation, snapshots.clone(), events.clone(), project_logs_cancel.clone()));
                    }
                    Command::CloseProjectLogs => project_logs_cancel.cancel(),
                    Command::PruneImages { mode, operation } => {
                        if pruning || !image_busy.is_empty() || !volume_busy.is_empty() || !compose_busy.is_empty() || !busy.is_empty() {
                            events.send(Event::ImagesPruned { operation, result: Err("Wait for the current Docker operation before pruning images".into()) }).await?;
                            continue;
                        }
                        pruning = true;
                        let docker = docker.clone();
                        let cancellation = cancel.child_token();
                        prune_jobs.push(tokio::spawn(async move {
                            let result = tokio::select! {
                                _ = cancellation.cancelled() => Err("Docker disconnected; refresh to check image state".into()),
                                result = crate::images::prune(&docker, mode) => result.map_err(|error| format!("{error:#}")),
                            };
                            (operation, result)
                        }));
                    }
                    Command::ConfigureCompose { config } => {
                        let project = config.project.clone();
                        let result = (|| -> Result<compose::Config> {
                            if compose_busy.contains(&project) { bail!("Wait for the project operation before changing configuration"); }
                            let config = config.validated()?;
                            settings = compose::Settings::save(&compose::Settings::path()?, &connection.endpoint, config.clone())?;
                            Ok(config)
                        })().map_err(|error| format!("{error:#}"));
                        let _ = events.send(Event::ComposeConfigured { project, result }).await;
                        dirty = true;
                    }
                    Command::Compose { project, operation, action } => {
                        let conflict = pruning || !image_busy.is_empty() || !volume_busy.is_empty() || compose_busy.contains(&project) || busy.iter().any(|id| container_projects.get(id).and_then(Option::as_ref) == Some(&project));
                        let config = settings.config(&connection.endpoint, &project)
                            .or_else(|| project_configs.get(&project)).cloned();
                        if let Some(config) = config.filter(|_| !conflict) {
                            compose_busy.insert(project.clone());
                            let endpoint = connection.endpoint.clone();
                            let events = events.clone();
                            let cancellation = cancel.child_token();
                            compose_jobs.push(tokio::spawn(async move {
                                let result = compose::run(config, endpoint, action, operation, events, cancellation).await.map_err(|error| format!("{error:#}"));
                                (project, operation, action, result)
                            }));
                        } else {
                            let _ = events.send(Event::ComposeFinished { project, operation, result: Err(if conflict { "Another operation is running on this project" } else { "Associate and save this project's Compose configuration first" }.into()) }).await;
                        }
                    }
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
                    Command::Action { id, action } if !pruning && volume_busy.is_empty() && !busy.contains(&id) && !container_projects.get(&id).and_then(Option::as_ref).is_some_and(|project| compose_busy.contains(project)) => {
                        busy.insert(id.clone());
                        let _ = events.send(Event::ActionStarted { id: id.clone(), action }).await;
                        let docker = docker.clone();
                        actions.push(tokio::spawn(async move {
                            let result = perform(&docker, &id, action).await.map_err(|e| e.to_string());
                            (id, result)
                        }));
                    }
                    Command::Action { id, .. } => {
                        let _ = events.send(Event::ActionFinished { id, result: Err("Another operation is running on this container, its Compose project, or a volume".into()) }).await;
                    },
                    Command::Terminal { id, generation, shell, columns, rows } => {
                        terminal_cancel.cancel(); terminal_cancel = cancel.child_token();
                        let (tx, rx) = mpsc::unbounded_channel(); terminal = Some((generation, tx));
                        tokio::spawn(terminal_session(docker.clone(), id, generation, shell.command(), columns, rows, rx, events.clone(), terminal_cancel.clone()));
                    }
                    Command::FavoriteTerminal { id, generation, endpoint, target, command, columns, rows } => {
                        // A favorite never replaces another terminal, including one still connecting.
                        let available = endpoint == connection.endpoint && terminal.is_none() && !pruning && volume_busy.is_empty() && !busy.contains(&id) && !container_projects.get(&id).and_then(Option::as_ref).is_some_and(|p| compose_busy.contains(p));
                        if !available {
                            let _ = events.send(Event::TerminalEnded { generation, error: Some("Cannot run favorite: terminal or Docker operation is active, or endpoint changed".into()) }).await;
                            continue;
                        }
                        let token = cancel.child_token();
                        terminal_cancel = token.clone();
                        let (tx, rx) = mpsc::unbounded_channel(); terminal = Some((generation, tx));
                        let docker = docker.clone(); let events = events.clone();
                        tokio::spawn(async move {
                            let valid = tokio::time::timeout(Duration::from_secs(15), docker.inspect_container(&id, None)).await
                                .ok().and_then(Result::ok).is_some_and(|info| {
                                    let labels = info.config.and_then(|c| c.labels).unwrap_or_default();
                                    let scope_matches = match target {
                                        crate::favorites::Target::Service { project, service } => labels.get("com.docker.compose.project") == Some(&project) && labels.get("com.docker.compose.service") == Some(&service),
                                        crate::favorites::Target::Container { name } => info.name.as_deref().map(|n| n.trim_start_matches('/')) == Some(name.as_str()) && !labels.contains_key("com.docker.compose.project"),
                                    };
                                    scope_matches && info.state.and_then(|s| s.running) == Some(true)
                                });
                            if token.is_cancelled() { return; }
                            if !valid {
                                let _ = events.send(Event::TerminalEnded { generation, error: Some("Favorite target is stopped, absent, changed, or unavailable".into()) }).await;
                                return;
                            }
                            terminal_session(docker, id, generation, vec!["sh".into(), "-c".into(), command], columns, rows, rx, events, token).await;
                        });
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
    // Load the finite history before painting it. Replaying history through the
    // live stream makes Follow visibly walk down the output on every selection.
    let started = chrono::Utc::now().timestamp() as i32;
    let history_options = LogsOptionsBuilder::default()
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .tail("500")
        .build();
    let mut history_stream = docker.logs(&id, Some(history_options));
    let mut history = LogHistory::default();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            frame = history_stream.next() => match frame {
                Some(Ok(frame)) => {
                    if let Err(error) = history.push(&frame.into_bytes()) {
                        let _ = events.send(Event::LogsEnded { generation, error: Some(error.to_string()) }).await;
                        return;
                    }
                },
                Some(Err(error)) => {
                    let _ = events.send(Event::LogsEnded { generation, error: Some(error.to_string()) }).await;
                    return;
                }
                None => break,
            }
        }
    }
    let since = history
        .latest
        .map_or(started, |timestamp| timestamp.timestamp() as i32);
    let data = std::mem::take(&mut history.data);
    tokio::select! {
        _ = cancel.cancelled() => return,
        result = events.send(Event::Log { generation, data }) => {
            if result.is_err() { return; }
        }
    }
    // Resume from the snapshot's timestamp so output produced between requests
    // is retained. Docker's second-resolution `since` also replays history;
    // remove those records, including duplicates at the exact boundary.
    let options = LogsOptionsBuilder::default()
        .follow(true)
        .stdout(true)
        .stderr(true)
        .timestamps(true)
        .since(since)
        .tail("all")
        .build();
    let mut stream = docker.logs(&id, Some(options));
    let error = loop {
        tokio::select! {
            _ = cancel.cancelled() => break None,
            frame = stream.next() => match frame {
                Some(Ok(frame)) => {
                    let data = history.live_data(&frame.into_bytes());
                    if !data.is_empty() && events.send(Event::Log { generation, data }).await.is_err() { return; }
                }
                Some(Err(error)) => break Some(error.to_string()),
                None => break None,
            }
        }
    };
    let _ = events.send(Event::LogsEnded { generation, error }).await;
}

#[derive(Default)]
struct LogHistory {
    data: Vec<u8>,
    latest: Option<chrono::DateTime<chrono::FixedOffset>>,
    boundary: HashMap<Vec<u8>, usize>,
}

impl LogHistory {
    // Docker's tail limit counts records, not bytes. Refuse oversized history
    // before copying arbitrarily large records into the initial snapshot.
    const MAX_BYTES: usize = 32 * 1024 * 1024;

    fn timestamp(record: &[u8]) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        let end = record.iter().position(|byte| *byte == b' ')?;
        chrono::DateTime::parse_from_rfc3339(std::str::from_utf8(&record[..end]).ok()?).ok()
    }

    fn push(&mut self, data: &[u8]) -> Result<()> {
        if data.len() > Self::MAX_BYTES.saturating_sub(self.data.len()) {
            bail!("Container log history exceeds 32 MiB; initial logs could not be loaded");
        }
        self.data.extend_from_slice(data);
        for record in data.split_inclusive(|byte| *byte == b'\n') {
            let Some(timestamp) = Self::timestamp(record) else {
                continue;
            };
            if self.latest.is_none_or(|latest| timestamp > latest) {
                self.latest = Some(timestamp);
                self.boundary.clear();
            }
            if self.latest == Some(timestamp) {
                *self.boundary.entry(record.to_vec()).or_default() += 1;
            }
        }
        Ok(())
    }

    fn live_data(&mut self, data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        for record in data.split_inclusive(|byte| *byte == b'\n') {
            if let (Some(timestamp), Some(latest)) = (Self::timestamp(record), self.latest) {
                if timestamp < latest {
                    continue;
                }
                if timestamp == latest
                    && let Some(remaining) = self.boundary.get_mut(record)
                    && *remaining > 0
                {
                    *remaining -= 1;
                    continue;
                }
            }
            output.extend_from_slice(record);
        }
        output
    }
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
    command: Vec<String>,
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
                bollard::models::ExecConfig {
                    // Fast commands can query their PTY before the post-start resize.
                    console_size: Some(vec![usize::from(rows), usize::from(columns)]),
                    attach_stdin: Some(true),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(true),
                    cmd: Some(command),
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
        // ConsoleSize initializes the PTY before execution. A post-start resize
        // can race a short-lived favorite and fail before its output is read.
        let _ = events.send(Event::TerminalReady { generation }).await;
        let mut completed = false;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => { let _ = input.shutdown().await; break; },
                frame = output.next() => match frame {
                    Some(Ok(frame)) => { events.send(Event::TerminalData { generation, data: frame.into_bytes().to_vec() }).await?; }
                    Some(Err(error)) => return Err(error.into()),
                    None => { completed = true; break; },
                },
                command = commands.recv() => match command {
                    Some(TerminalCommand::Input(data)) => { input.write_all(&data).await?; input.flush().await?; },
                    Some(TerminalCommand::Resize(columns, rows)) => {
                        if let Err(error) = docker.resize_exec(&exec.id, ResizeExecOptions { height: rows, width: columns }).await {
                            // A completed command still has buffered output to drain.
                            if docker.inspect_exec(&exec.id).await?.running != Some(false) { return Err(error.into()); }
                        }
                    },
                    None => { let _ = input.shutdown().await; break; },
                }
            }
        }
        if completed {
            let info = time::timeout(Duration::from_secs(3), docker.inspect_exec(&exec.id))
                .await
                .context("Could not read terminal exit status in time")??;
            if let Some(code) = info.exit_code.filter(|code| *code != 0) {
                bail!("Command exited with status {code}");
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    let error = tokio::select! { _ = cancel.cancelled() => None, result = result => result.err().map(|error| error.to_string()) };
    let _ = events
        .send(Event::TerminalEnded { generation, error })
        .await;
}

#[cfg(test)]
mod log_history_tests {
    use super::LogHistory;

    #[test]
    fn resume_keeps_gap_output_and_removes_snapshot_replay() {
        let old = b"2026-10-13T05:00:00.100000000Z old\n";
        let boundary = b"2026-10-13T05:00:00.900000000Z repeated\n";
        let gap = b"2026-10-13T05:00:00.950000000Z between requests\n";
        let same_time_new = b"2026-10-13T05:00:00.900000000Z new stderr\n";
        let mut history = LogHistory::default();
        history.push(old).unwrap();
        history.push(boundary).unwrap();
        history.push(boundary).unwrap();
        assert_eq!(history.data, [old.as_slice(), boundary, boundary].concat());
        assert!(history.live_data(old).is_empty());
        assert!(history.live_data(boundary).is_empty());
        assert!(history.live_data(boundary).is_empty());
        assert_eq!(history.live_data(same_time_new), same_time_new);
        assert_eq!(history.live_data(gap), gap);
        // Equal payloads with the same timestamp still preserve multiplicity.
        assert_eq!(history.live_data(boundary), boundary);
    }

    #[test]
    fn oversized_history_is_rejected_before_mutating_resume_state() {
        let mut history = LogHistory::default();
        history
            .push(b"2026-10-13T05:00:00.100000000Z retained\n")
            .unwrap();
        let retained = history.data.clone();
        let latest = history.latest;
        let boundary = history.boundary.clone();
        let oversized = vec![b'x'; LogHistory::MAX_BYTES];
        assert!(history.push(&oversized).is_err());
        assert_eq!(history.data, retained);
        assert_eq!(history.latest, latest);
        assert_eq!(history.boundary, boundary);

        let remaining = LogHistory::MAX_BYTES - history.data.len();
        history.push(&oversized[..remaining / 2]).unwrap();
        history
            .push(&oversized[..remaining - remaining / 2])
            .unwrap();
        assert_eq!(history.data.len(), LogHistory::MAX_BYTES);
        assert!(history.push(b"x").is_err());
    }
}
