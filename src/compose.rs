//! Compose configuration belongs to a project on a particular Docker endpoint.
//! Labels suggest paths; only an explicitly saved association enables controls.
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
        args.extend(action.args().iter().map(|arg| (*arg).into()));
        args
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Start => &["up", "--detach", "--no-build"],
            Self::Stop => &["stop"],
            Self::Restart => &["restart"],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub config: Option<Config>,
    pub hint: Config,
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
        std::fs::create_dir_all(path.parent().context("Configuration path has no parent")?)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(&settings)?)?;
        std::fs::rename(&temporary, path)?;
        Ok(settings)
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
    let mut command = crate::docker_cli::command();
    command
        .args(config.args(&endpoint, action))
        .current_dir(&config.directory)
        .env_remove("DOCKER_CONTEXT")
        .env("DOCKER_HOST", &endpoint)
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
}
