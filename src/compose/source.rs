//! Read-only local source metadata. Docker labels remain the source of running
//! projects; a saved association supplies it only for containerless projects.
use super::Project;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

impl Project {
    pub fn source_files(&self) -> Vec<PathBuf> {
        self.hint
            .files
            .iter()
            .map(|file| {
                if file.is_absolute() {
                    file.clone()
                } else {
                    self.hint.directory.join(file)
                }
            })
            .collect()
    }

    pub fn source_label(&self) -> String {
        let files = self.source_files();
        match files.first() {
            Some(file) if files.len() > 1 => {
                format!(
                    "{} (+{} override{})",
                    file.display(),
                    files.len() - 1,
                    if files.len() == 2 { "" } else { "s" }
                )
            }
            Some(file) => file.display().to_string(),
            None => "Compose file path unavailable".into(),
        }
    }

    pub fn source_tooltip(&self) -> String {
        let files = self.source_files();
        let mut text = if files.is_empty() {
            "Compose file path unavailable".into()
        } else {
            format!(
                "Compose files (in order):\n{}",
                files
                    .iter()
                    .map(|file| file.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        if let Some(root) = &self.worktree {
            text.push_str(&format!("\nGit worktree: {}", root.display()));
        }
        text
    }

    pub fn discover_local_source(&mut self) {
        self.worktree = self
            .source_files()
            .first()
            .filter(|file| file.is_absolute() && file.is_file())
            .and_then(|file| file.parent())
            .and_then(linked_worktree);
        if self.config.is_none() {
            self.config = self.hint.validated().ok();
        }
    }
}

fn small_text(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(8193)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 8192 {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn resolve(base: &Path, value: &str) -> Option<PathBuf> {
    base.join(value.trim()).canonicalize().ok()
}

/// Verify Git's linked-worktree metadata, including the back pointer. A .git
/// file alone also occurs in submodules and separate-git-dir repositories.
fn linked_worktree(directory: &Path) -> Option<PathBuf> {
    let directory = directory.canonicalize().ok()?;
    for root in directory.ancestors().take(128) {
        let marker = root.join(".git");
        if marker.is_dir() {
            return None;
        }
        let Some(contents) = small_text(&marker) else {
            continue;
        };
        let admin = resolve(root, contents.trim().strip_prefix("gitdir: ")?)?;
        let common = resolve(&admin, &small_text(&admin.join("commondir"))?)?;
        if admin.parent()? != common.join("worktrees").canonicalize().ok()? {
            return None;
        }
        let back_pointer = resolve(&admin, &small_text(&admin.join("gitdir"))?)?;
        return (back_pointer == marker.canonicalize().ok()?).then(|| root.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::Config;
    use super::*;

    #[test]
    fn source_paths_preserve_labels_and_override_order() {
        let project = Project {
            name: "demo".into(),
            config: Some(Config {
                directory: "/other".into(),
                files: vec!["other.yaml".into()],
                ..Default::default()
            }),
            hint: Config {
                directory: "/checkout".into(),
                files: vec!["compose.yaml".into(), "/overrides/dev.yaml".into()],
                ..Default::default()
            },
            worktree: None,
        };
        assert_eq!(
            project.source_files(),
            vec![
                PathBuf::from("/checkout/compose.yaml"),
                PathBuf::from("/overrides/dev.yaml")
            ]
        );
        assert_eq!(
            project.source_label(),
            "/checkout/compose.yaml (+1 override)"
        );
        assert!(
            project
                .source_tooltip()
                .contains("/checkout/compose.yaml\n/overrides/dev.yaml")
        );
    }

    #[test]
    fn verifies_real_linked_worktrees_and_rejects_other_git_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let checkout = temp.path().join("linked checkout");
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet", repo.to_str().unwrap()]);
        git(&[
            "-C",
            repo.to_str().unwrap(),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "fixture",
        ]);
        git(&[
            "-C",
            repo.to_str().unwrap(),
            "worktree",
            "add",
            "--quiet",
            "--detach",
            checkout.to_str().unwrap(),
        ]);
        let nested = checkout.join("docker/dev");
        std::fs::create_dir_all(&nested).unwrap();
        let compose_file = nested.join("compose.yaml");
        let override_file = nested.join("compose.dev.yaml");
        std::fs::write(&compose_file, "services: {}\n").unwrap();
        std::fs::write(&override_file, "services: {}\n").unwrap();
        let mut project = Project {
            name: "demo".into(),
            config: None,
            hint: Config {
                project: "demo".into(),
                directory: nested.clone(),
                files: vec!["compose.yaml".into(), "compose.dev.yaml".into()],
                ..Default::default()
            },
            worktree: None,
        };
        project.discover_local_source();
        assert_eq!(project.worktree, Some(checkout.canonicalize().unwrap()));
        assert_eq!(
            project.config.as_ref().unwrap().files,
            vec![
                compose_file.canonicalize().unwrap(),
                override_file.canonicalize().unwrap()
            ]
        );
        // Explicit choices survive discovery, even when labels differ.
        let saved = Config {
            profiles: vec!["dev".into()],
            ..project.config.clone().unwrap()
        };
        project.config = Some(saved.clone());
        project.discover_local_source();
        assert_eq!(project.config, Some(saved));
        project.config = None;
        std::fs::remove_file(&override_file).unwrap();
        project.discover_local_source();
        assert!(project.config.is_none());
        project.hint.env_files = vec!["missing.env".into()];
        project.hint.files.pop();
        project.discover_local_source();
        assert!(project.config.is_none());
        assert_eq!(
            linked_worktree(&nested),
            Some(checkout.canonicalize().unwrap())
        );
        assert_eq!(linked_worktree(&repo), None);
        assert_eq!(linked_worktree(&temp.path().join("missing")), None);
        let separate = temp.path().join("separate");
        git(&[
            "init",
            "--quiet",
            "--separate-git-dir",
            temp.path().join("git-data").to_str().unwrap(),
            separate.to_str().unwrap(),
        ]);
        assert_eq!(linked_worktree(&separate), None);
        std::fs::write(nested.join(".git"), "gitdir: /missing/admin\n").unwrap();
        assert_eq!(linked_worktree(&nested), None);
    }
}
