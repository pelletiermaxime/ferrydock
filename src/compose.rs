//! Compose configuration belongs to a project on a particular Docker endpoint.
//! Accessible labeled files associate automatically; saved configurations win.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, sync::mpsc};
use tokio_util::sync::CancellationToken;

use crate::{docker::Event, model::Container};

mod cleanup;
mod source;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub project: String,
    pub directory: PathBuf,
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub env_files: Vec<PathBuf>,
    #[serde(default)]
    pub profiles: Vec<String>,
}

impl Config {
    pub fn validated(&self) -> Result<Self> {
        if self.project.is_empty()
            || !self
                .project
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
            || !self
                .project
                .starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            bail!("Invalid Compose project name");
        }
        if !self.directory.is_absolute() {
            bail!("Project directory must be an absolute path");
        }
        let directory = self
            .directory
            .canonicalize()
            .context("Project directory is unavailable")?;
        if !directory.is_dir() {
            bail!("Project directory must be a directory");
        }
        if self.files.is_empty() {
            bail!("Select at least one Compose file");
        }
        let resolve = |paths: &[PathBuf]| -> Result<Vec<PathBuf>> {
            paths
                .iter()
                .map(|path| {
                    let path = if path.is_absolute() {
                        path.clone()
                    } else {
                        directory.join(path)
                    };
                    let path = path
                        .canonicalize()
                        .with_context(|| format!("File unavailable: {}", path.display()))?;
                    if !path.is_file() {
                        bail!("Not a file: {}", path.display());
                    }
                    Ok(path)
                })
                .collect()
        };
        Ok(Self {
            project: self.project.clone(),
            directory: directory.clone(),
            files: resolve(&self.files)?,
            env_files: resolve(&self.env_files)?,
            profiles: self.profiles.clone(),
        })
    }

    pub fn args(&self, endpoint: &str, action: Action) -> Vec<String> {
        let mut args = vec![
            "--host".into(),
            endpoint.into(),
            "compose".into(),
            "--ansi".into(),
            "never".into(),
            "--project-name".into(),
            self.project.clone(),
            "--project-directory".into(),
            self.directory.to_string_lossy().into_owned(),
        ];
        for file in &self.files {
            args.extend(["--file".into(), file.to_string_lossy().into_owned()]);
        }
        for file in &self.env_files {
            args.extend(["--env-file".into(), file.to_string_lossy().into_owned()]);
        }
        for profile in &self.profiles {
            args.extend(["--profile".into(), profile.clone()]);
        }
        if action.is_cleanup() {
            args.extend(["--profile".into(), "*".into()]);
        }
        args.extend(action.args().iter().map(|arg| (*arg).into()));
        args
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Rebuild,
    Cleanup { volumes: bool, images: bool },
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Rebuild => "Rebuild & restart",
            Self::Cleanup { .. } => "Stop & cleanup",
        }
    }
    pub fn is_cleanup(self) -> bool {
        matches!(self, Self::Cleanup { .. })
    }
    fn args(self) -> Vec<&'static str> {
        match self {
            Self::Start => vec!["up", "--detach", "--no-build"],
            Self::Stop => vec!["stop"],
            Self::Restart => vec!["restart"],
            Self::Rebuild => vec!["up", "--detach", "--build", "--force-recreate"],
            Self::Cleanup { volumes, .. } => {
                let mut args = vec!["down", "--remove-orphans"];
                if volumes {
                    args.push("--volumes");
                }
                args
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub config: Option<Config>,
    pub hint: Config,
    /// Verified linked checkout containing the primary Compose file.
    pub worktree: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Association {
    endpoint: String,
    config: Config,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    associations: Vec<Association>,
}
impl Settings {
    pub fn path() -> Result<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .context("Neither XDG_CONFIG_HOME nor HOME is available")?;
        Ok(base.join("ferrydock/compose.json"))
    }
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("Cannot read saved Compose associations")
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn config(&self, endpoint: &str, project: &str) -> Option<&Config> {
        self.associations
            .iter()
            .find(|entry| entry.endpoint == endpoint && entry.config.project == project)
            .map(|entry| &entry.config)
    }
    pub fn save(path: &Path, endpoint: &str, config: Config) -> Result<Self> {
        // Reload before writing so an unreadable file is never silently replaced.
        let mut settings = Self::load(path)?;
        settings
            .associations
            .retain(|entry| entry.endpoint != endpoint || entry.config.project != config.project);
        settings.associations.push(Association {
            endpoint: endpoint.into(),
            config,
        });
        settings.persist(path)
    }
    pub fn forget(path: &Path, endpoint: &str, project: &str) -> Result<Self> {
        let mut settings = Self::load(path)?;
        settings
            .associations
            .retain(|entry| entry.endpoint != endpoint || entry.config.project != project);
        settings.persist(path)
    }
    fn persist(self, path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path.parent().context("Configuration path has no parent")?)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self)?)?;
        std::fs::rename(&temporary, path)?;
        Ok(self)
    }
    pub fn projects(&self, endpoint: &str, containers: &[Container]) -> Vec<Project> {
        let mut projects = BTreeMap::new();
        for container in containers {
            if let Some(name) = &container.project {
                projects.entry(name.clone()).or_insert_with(|| Project {
                    name: name.clone(),
                    config: self.config(endpoint, name).cloned(),
                    hint: Config {
                        project: name.clone(),
                        directory: container.compose_directory.clone(),
                        files: container.compose_files.clone(),
                        env_files: container.compose_env_files.clone(),
                        profiles: vec![],
                    },
                    worktree: None,
                });
            }
        }
        for association in self
            .associations
            .iter()
            .filter(|entry| entry.endpoint == endpoint)
        {
            projects
                .entry(association.config.project.clone())
                .or_insert_with(|| Project {
                    name: association.config.project.clone(),
                    config: Some(association.config.clone()),
                    hint: association.config.clone(),
                    worktree: None,
                });
        }
        projects.into_values().collect()
    }
}

pub async fn run(
    config: Config,
    endpoint: String,
    action: Action,
    operation: u64,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) -> Result<()> {
    let config = config.validated()?;
    let images = if matches!(action, Action::Cleanup { images: true, .. }) {
        cleanup::generated_images(&config, &endpoint, &cancel).await?
    } else {
        Vec::new()
    };
    run_process(
        &config,
        &endpoint,
        config.args(&endpoint, action),
        action,
        operation,
        &events,
        &cancel,
    )
    .await?;
    if !images.is_empty() {
        let mut args = vec![
            "--host".into(),
            endpoint.clone(),
            "image".into(),
            "rm".into(),
        ];
        args.extend(images);
        run_process(
            &config, &endpoint, args, action, operation, &events, &cancel,
        )
        .await?;
    }
    Ok(())
}

fn command(config: &Config, endpoint: &str, args: Vec<String>) -> tokio::process::Command {
    let mut command = crate::docker_cli::command();
    command
        .args(args)
        .current_dir(&config.directory)
        .env_remove("DOCKER_CONTEXT")
        .env("DOCKER_HOST", endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Saved configuration is authoritative; ambient Compose overrides would
    // otherwise silently change profiles, files or project scope.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("COMPOSE_") {
            command.env_remove(name);
        }
    }
    command
}

async fn run_process(
    config: &Config,
    endpoint: &str,
    args: Vec<String>,
    action: Action,
    operation: u64,
    events: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut command = command(config, endpoint, args);
    let mut child = command
        .spawn()
        .context("Cannot launch Docker Compose v2; install the docker compose plugin")?;
    let mut stdout = child.stdout.take().context("Missing Compose stdout")?;
    let mut stderr = child.stderr.take().context("Missing Compose stderr")?;
    let mut out = [0; 4096];
    let mut err = [0; 4096];
    let (mut out_done, mut err_done) = (false, false);
    while !out_done || !err_done {
        let (count, bytes) = tokio::select! {
            _ = cancel.cancelled() => { child.kill().await?; child.wait().await?; bail!("Compose operation interrupted; Docker changes are not rolled back"); },
            read = stdout.read(&mut out), if !out_done => { let count = read?; out_done = count == 0; (count, &out[..count]) },
            read = stderr.read(&mut err), if !err_done => { let count = read?; err_done = count == 0; (count, &err[..count]) },
        };
        if count > 0 {
            tokio::select! {
                _ = cancel.cancelled() => { child.kill().await?; child.wait().await?; bail!("Compose operation interrupted"); },
                sent = events.send(Event::ComposeOutput { project: config.project.clone(), operation, data: bytes.to_vec() }) => { sent?; },
            }
        }
    }
    let status = tokio::select! {
        _ = cancel.cancelled() => { child.kill().await?; child.wait().await?; bail!("Compose operation interrupted"); },
        status = child.wait() => status?,
    };
    if !status.success() {
        bail!(
            "Compose {} failed ({status}); see operation output",
            action.label()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_order_and_endpoint_without_shell_interpolation() {
        let config = Config {
            project: "demo".into(),
            directory: "/tmp/project space".into(),
            files: vec!["base.yaml".into(), "override.yaml".into()],
            env_files: vec![".env.local".into()],
            profiles: vec!["dev".into()],
        };
        let args = config.args("unix:///tmp/daemon.sock", Action::Start);
        assert_eq!(
            &args[..3],
            &["--host", "unix:///tmp/daemon.sock", "compose"]
        );
        assert_eq!(
            args.iter().position(|a| a == "base.yaml").unwrap() + 2,
            args.iter().position(|a| a == "override.yaml").unwrap()
        );
        assert!(args.contains(&"/tmp/project space".into()));
        assert_eq!(&args[args.len() - 3..], &["up", "--detach", "--no-build"]);
    }
    #[test]
    fn settings_are_endpoint_scoped_and_keep_containerless_projects() {
        let config = Config {
            project: "demo".into(),
            ..Default::default()
        };
        let settings = Settings {
            associations: vec![Association {
                endpoint: "unix:///a".into(),
                config,
            }],
        };
        assert_eq!(settings.projects("unix:///a", &[]).len(), 1);
        assert!(settings.projects("unix:///b", &[]).is_empty());
    }
    #[test]
    fn cleanup_is_project_scoped_and_respects_retention_options() {
        let config = Config {
            project: "owned".into(),
            directory: "/tmp/owned".into(),
            files: vec!["compose.yaml".into()],
            profiles: vec!["extra".into()],
            ..Default::default()
        };
        for volumes in [false, true] {
            for images in [false, true] {
                let args = config.args("unix:///owned.sock", Action::Cleanup { volumes, images });
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--project-name", "owned"])
                );
                assert!(args.windows(2).any(|pair| pair == ["--profile", "*"]));
                assert!(args.contains(&"down".into()) && args.contains(&"--remove-orphans".into()));
                assert_eq!(args.contains(&"--volumes".into()), volumes);
                // Image removal is a separate explicit-name step. Compose's
                // --rmi local also deletes custom-tagged builds in some versions.
                assert!(!args.contains(&"--rmi".into()));
                assert!(!args.contains(&"all".into()));
            }
        }
    }
    #[test]
    fn forgetting_association_preserves_other_endpoints_and_projects() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("compose.json");
        let config = |project: &str| Config {
            project: project.into(),
            ..Default::default()
        };
        Settings::save(&path, "unix:///a", config("owned")).unwrap();
        Settings::save(&path, "unix:///b", config("owned")).unwrap();
        Settings::save(&path, "unix:///a", config("other")).unwrap();
        let settings = Settings::forget(&path, "unix:///a", "owned").unwrap();
        assert!(settings.config("unix:///a", "owned").is_none());
        assert!(settings.config("unix:///b", "owned").is_some());
        assert!(settings.config("unix:///a", "other").is_some());
    }
}
