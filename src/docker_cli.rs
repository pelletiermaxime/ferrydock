//! Finder launches do not inherit a shell's Docker/Homebrew PATH setup.
use tokio::process::Command;

pub(crate) fn command() -> Command {
    #[cfg(target_os = "macos")]
    {
        macos_command(std::env::var_os("PATH"), std::env::var_os("HOME"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[allow(unused_mut)]
        let mut command = Command::new("docker");
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW for background CLI work.
        command
    }
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn macos_command(path: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Command {
    use std::{env, path::PathBuf};

    let mut command = Command::new("docker");
    let mut directories = path
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(home) = home {
        let home = PathBuf::from(home);
        directories.push(home.join(".docker/bin"));
        directories.push(home.join("Applications/Docker.app/Contents/Resources/bin"));
    }
    directories.extend(
        [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/Applications/Docker.app/Contents/Resources/bin",
            "/usr/bin",
            "/bin",
        ]
        .map(PathBuf::from),
    );
    if let Ok(path) = env::join_paths(directories) {
        // Extend only the child environment. Compose's credential helpers and
        // other subprocesses need these paths too; the desktop process does not.
        command.env("PATH", path);
    }
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[tokio::test]
    async fn finder_path_finds_user_docker_and_preserves_explicit_path_priority() {
        let root = std::env::temp_dir().join(format!("ferrydock-cli-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        // Remove the fixture even when a test assertion fails.
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _cleanup = Cleanup(root.clone());
        let user_bin = root.join(".docker/bin");
        let explicit_bin = root.join("custom");
        for (directory, label) in [(&user_bin, "user"), (&explicit_bin, "explicit")] {
            fs::create_dir_all(directory).unwrap();
            let docker = directory.join("docker");
            fs::write(&docker, format!("#!/bin/sh\nprintf '{label}'\n")).unwrap();
            fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let parent_path = std::env::var_os("PATH");
        // Model Finder's system-only path without accidentally selecting the
        // Linux host's /usr/bin/docker when this regression test runs on Linux.
        let finder_bin = root.join("system-bin");
        fs::create_dir(&finder_bin).unwrap();
        let finder = macos_command(Some(finder_bin.into()), Some(root.clone().into()));
        let output = finder.into_std().output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"user");
        let explicit = macos_command(Some(explicit_bin.into()), Some(root.into()));
        let output = explicit.into_std().output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"explicit");
        assert_eq!(std::env::var_os("PATH"), parent_path);
    }
}
