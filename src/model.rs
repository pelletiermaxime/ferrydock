use std::collections::{BTreeMap, VecDeque};

use bollard::models::ContainerSummary;

#[derive(Clone, Debug)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub project: Option<String>,
    pub service: Option<String>,
    pub ports: String,
    pub compose_directory: std::path::PathBuf,
    pub compose_files: Vec<std::path::PathBuf>,
    pub compose_env_files: Vec<std::path::PathBuf>,
}

impl Container {
    pub fn running(&self) -> bool {
        self.state == "running"
    }

    pub fn from_summary(summary: ContainerSummary) -> Self {
        let id = summary.id.unwrap_or_default();
        let labels = summary.labels.unwrap_or_default();
        let name = summary
            .names
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_else(|| id.chars().take(12).collect())
            .trim_start_matches('/')
            .to_owned();
        let ports = summary
            .ports
            .unwrap_or_default()
            .into_iter()
            .filter_map(|port| {
                port.public_port
                    .map(|public| format!("{public} → {}", port.private_port))
            })
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            id,
            name,
            image: summary.image.unwrap_or_default(),
            state: summary
                .state
                .map(|state| state.to_string())
                .unwrap_or_default(),
            status: summary.status.unwrap_or_default(),
            project: labels
                .get("com.docker.compose.project")
                .filter(|s| !s.is_empty())
                .cloned(),
            service: labels
                .get("com.docker.compose.service")
                .filter(|s| !s.is_empty())
                .cloned(),
            ports,
            compose_directory: labels
                .get("com.docker.compose.project.working_dir")
                .map(std::path::PathBuf::from)
                .unwrap_or_default(),
            compose_files: labels
                .get("com.docker.compose.project.config_files")
                .map(|value| {
                    value
                        .split(',')
                        .filter(|part| !part.is_empty())
                        .map(std::path::PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            compose_env_files: labels
                .get("com.docker.compose.project.environment_file")
                .map(|value| {
                    value
                        .split(',')
                        .filter(|part| !part.is_empty())
                        .map(std::path::PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GroupKey {
    Compose(String),
    Standalone,
}

impl GroupKey {
    pub fn label(&self) -> &str {
        match self {
            Self::Compose(project) => project,
            Self::Standalone => "Standalone",
        }
    }
}

pub fn groups(
    containers: &[Container],
    running_only: bool,
    filter: &str,
) -> BTreeMap<GroupKey, Vec<Container>> {
    let filter = filter.to_lowercase();
    let mut groups = BTreeMap::<GroupKey, Vec<Container>>::new();
    for container in containers {
        if running_only && !container.running() {
            continue;
        }
        if !filter.is_empty()
            && ![
                &container.name,
                &container.image,
                container.project.as_deref().unwrap_or(""),
                container.service.as_deref().unwrap_or(""),
            ]
            .iter()
            .any(|value| value.to_lowercase().contains(&filter))
        {
            continue;
        }
        groups
            .entry(
                container
                    .project
                    .clone()
                    .map(GroupKey::Compose)
                    .unwrap_or(GroupKey::Standalone),
            )
            .or_default()
            .push(container.clone());
    }
    for members in groups.values_mut() {
        members.sort_by(|a, b| a.name.cmp(&b.name));
    }
    groups
}

/// Bounded lines plus an incremental UTF-8 decoder: Docker frames need not end
/// at a character or line boundary.
#[derive(Default)]
pub struct LogBuffer {
    pub lines: VecDeque<String>,
    pub partial: String,
    utf8: Vec<u8>,
}

impl LogBuffer {
    pub const MAX_LINES: usize = 1500;
    const MAX_LINE_BYTES: usize = 65536;

    pub fn push(&mut self, data: &[u8]) {
        self.utf8.extend_from_slice(data);
        let mut text = String::new();
        let mut consumed = 0;
        loop {
            match std::str::from_utf8(&self.utf8[consumed..]) {
                Ok(decoded) => {
                    text.push_str(decoded);
                    consumed = self.utf8.len();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(
                        std::str::from_utf8(&self.utf8[consumed..consumed + valid]).unwrap(),
                    );
                    consumed += valid;
                    if let Some(length) = error.error_len() {
                        text.push('�');
                        consumed += length;
                    } else {
                        break;
                    }
                }
            }
        }
        self.utf8.drain(..consumed);
        // Cap individual lines as well as history, including programs that
        // continuously write without a newline.
        for part in text.split_inclusive('\n') {
            self.partial.push_str(part);
            if self.partial.len() > Self::MAX_LINE_BYTES {
                let mut cut = Self::MAX_LINE_BYTES;
                while !self.partial.is_char_boundary(cut) {
                    cut -= 1;
                }
                self.partial.truncate(cut);
                self.partial.push_str(" … [line truncated]\n");
            }
            if self.partial.ends_with('\n') {
                self.lines.push_back(
                    std::mem::take(&mut self.partial)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned(),
                );
            }
        }
        while self.lines.len() > Self::MAX_LINES {
            self.lines.pop_front();
        }
    }

    pub fn text(&self) -> String {
        let mut text = self.lines.iter().cloned().collect::<Vec<_>>().join("\n");
        if !self.partial.is_empty() {
            text.push('\n');
            text.push_str(&self.partial);
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_utf8_recovers_without_losing_a_split_character() {
        let mut logs = LogBuffer::default();
        logs.push(b"valid\xff\xfe\xc3(");
        logs.push(b" caf\xc3");
        logs.push(b"\xa9\n");
        assert_eq!(logs.lines[0], "valid���( café");
        assert!(logs.utf8.is_empty());
        logs.push(&[vec![0xff; 10_000], b"\n".to_vec()].concat());
        assert_eq!(logs.lines[1], "�".repeat(10_000));
    }

    #[test]
    fn logs_decode_split_characters_and_bound_history() {
        let mut logs = LogBuffer::default();
        logs.push(b"caf\xc3");
        logs.push(b"\xa9\n");
        assert_eq!(logs.lines[0], "café");
        for _ in 0..2000 {
            logs.push(b"entry\n");
        }
        assert_eq!(logs.lines.len(), LogBuffer::MAX_LINES);
        logs.push(&vec![b'x'; 100_000]);
        assert!(logs.partial.len() <= LogBuffer::MAX_LINE_BYTES);
    }

    #[test]
    fn replicas_are_preserved_and_projects_filter_by_label() {
        let summary = |id: &str, name: &str| {
            Container::from_summary(ContainerSummary {
                id: Some(id.into()),
                names: Some(vec![format!("/{name}")]),
                labels: Some(
                    [
                        ("com.docker.compose.project".into(), "demo".into()),
                        ("com.docker.compose.service".into(), "web".into()),
                    ]
                    .into(),
                ),
                state: Some(bollard::models::ContainerSummaryStateEnum::RUNNING),
                ..Default::default()
            })
        };
        let containers = vec![summary("one", "demo-web-1"), summary("two", "demo-web-2")];
        assert_eq!(
            groups(&containers, true, "demo")[&GroupKey::Compose("demo".into())].len(),
            2
        );
        assert_eq!(
            groups(&containers, true, "web")[&GroupKey::Compose("demo".into())].len(),
            2
        );
        assert!(groups(&containers, true, "missing").is_empty());
    }

    #[test]
    fn standalone_project_name_does_not_merge_unmanaged_containers() {
        let composed = Container::from_summary(ContainerSummary {
            id: Some("compose".into()),
            names: Some(vec!["/compose".into()]),
            labels: Some([("com.docker.compose.project".into(), "Standalone".into())].into()),
            ..Default::default()
        });
        let unmanaged = Container::from_summary(ContainerSummary {
            id: Some("unmanaged".into()),
            names: Some(vec!["/unmanaged".into()]),
            ..Default::default()
        });
        let groups = groups(&[composed, unmanaged], false, "");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[&GroupKey::Standalone][0].id, "unmanaged");
        assert_eq!(
            groups[&GroupKey::Compose("Standalone".into())][0].id,
            "compose"
        );
    }
}
