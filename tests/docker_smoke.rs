use std::{env, time::Duration};

use anyhow::{Context, Result, bail};
use bollard::query_parameters::LogsOptionsBuilder;
use ferrydock::{
    docker::{self, Action, Backend, Command, Event, Shell},
    terminal::TerminalCore,
};
use futures_util::StreamExt;
use tokio::{sync::mpsc, time};

async fn receive(events: &mut mpsc::Receiver<Event>) -> Result<Event> {
    time::timeout(Duration::from_secs(20), events.recv())
        .await
        .context("Timed out waiting for Docker event")?
        .context("Docker worker ended")
}

async fn wait_for_container_state(docker: &bollard::Docker, id: &str, running: bool) -> Result<()> {
    let mut last_state = String::new();
    let result = time::timeout(Duration::from_secs(20), async {
        loop {
            let containers = docker::snapshot(docker).await?;
            let probe = containers
                .iter()
                .find(|container| container.id == id)
                .context("Fixture disappeared")?;
            last_state = format!("{} ({})", probe.state, probe.status);
            if probe.running() == running {
                return Ok(());
            }
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    result.with_context(|| {
        format!("Timed out waiting for running={running}; last Docker state: {last_state}")
    })?
}

async fn wait_for_text(
    events: &mut mpsc::Receiver<Event>,
    terminal: &mut TerminalCore,
    generation: u64,
    expected_line: &str,
) -> Result<()> {
    let result = time::timeout(Duration::from_secs(20), async {
        loop {
            match receive(events).await? {
                Event::TerminalData {
                    generation: current,
                    data,
                } if current == generation => {
                    terminal.feed(&data);
                    if terminal
                        .visible_text()
                        .lines()
                        .any(|line| line.trim() == expected_line)
                    {
                        return Ok(());
                    }
                }
                Event::TerminalEnded {
                    generation: current,
                    error,
                } if current == generation => {
                    bail!("Terminal ended before expected output: {error:?}")
                }
                _ => {}
            }
        }
    })
    .await;
    if result.is_err() {
        eprintln!("Last fixture terminal screen:\n{}", terminal.visible_text());
    }
    result.context("Terminal output timed out")?
}

#[tokio::test]
#[ignore = "Operates only on scripts/verify-docker.sh's owned disposable fixture"]
async fn docker_external_lifecycle_markers() -> Result<()> {
    let id = env::var("FERRYDOCK_TEST_CONTAINER").context("Run scripts/verify-docker.sh")?;
    let connection = docker::resolve_connection().await?;
    let docker = docker::connect(&connection).await?;
    let fixture = docker.inspect_container(&id, None).await?;
    let labels = fixture
        .config
        .and_then(|config| config.labels)
        .unwrap_or_default();
    if labels.get("dev.ferrydock.fixture").map(String::as_str) != Some("true")
        || !fixture
            .name
            .as_deref()
            .unwrap_or("")
            .starts_with("/ferrydock-verify-")
    {
        bail!("Refusing lifecycle actions on an unowned container");
    }
    let project = labels
        .get("com.docker.compose.project")
        .context("Fixture project label missing")?;
    let (_backend, mut events) = Backend::start();
    let mut session = None;
    time::timeout(Duration::from_secs(20), async {
        loop {
            match receive(&mut events).await? {
                Event::LifecycleSession(value) => session = Some(value),
                Event::Snapshot(containers)
                    if session.is_some() && containers.iter().any(|c| c.id == id) =>
                {
                    return Ok::<_, anyhow::Error>(());
                }
                Event::Disconnected(error) => bail!("Worker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await??;
    let mut observations = Vec::new();
    let mut restart_summary = None;
    let mut stop_summary = None;
    for (arguments, expected) in [
        (vec!["stop", "--time", "1"], "stop"),
        (vec!["start"], "start"),
        (vec!["restart", "--time", "1"], "restart"),
    ] {
        let mut operation_markers = std::collections::VecDeque::new();
        // A separate Docker client, never a FerryDock Command::Action.
        let output = time::timeout(
            Duration::from_secs(20),
            tokio::process::Command::new("docker")
                .args(["--host", &connection.endpoint])
                .args(arguments)
                .arg(&id)
                .output(),
        )
        .await??;
        if !output.status.success() {
            bail!(
                "Fixture CLI failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        time::timeout(Duration::from_secs(20), async {
            loop {
                if let Event::Lifecycle {
                    session: current,
                    marker,
                } = receive(&mut events).await?
                    && Some(current) == session
                    && marker.container == id
                {
                    assert_eq!(marker.project.as_deref(), Some(project.as_str()));
                    assert_eq!(marker.service, "probe");
                    observations.push(marker.text());
                    if expected == "stop"
                        && !matches!(marker.action.as_str(), "kill" | "stop" | "die")
                    {
                        continue;
                    }
                    let mut complete = marker.action == expected;
                    operation_markers.push_back(marker);
                    if expected == "stop" {
                        complete = operation_markers
                            .iter()
                            .any(|marker| marker.action == "stop")
                            && operation_markers
                                .iter()
                                .any(|marker| marker.action == "die");
                    }
                    if complete {
                        return Ok::<_, anyhow::Error>(());
                    }
                }
            }
        })
        .await
        .context("External lifecycle event was not observed")??;
        if expected == "stop" {
            let summary = ferrydock::lifecycle::summarize(&operation_markers);
            assert_eq!(
                summary
                    .iter()
                    .map(|m| m.message.as_str())
                    .collect::<Vec<_>>(),
                ["Container stop requested", "Container stopped"]
            );
            assert!(summary[1].details.contains("exit code:"));
            stop_summary = Some(
                serde_json::json!({"visible_markers":summary.len(),"raw_events":operation_markers.len(),"messages":summary.iter().map(|m| m.message.as_str()).collect::<Vec<_>>(),"details":summary[1].details}),
            );
        }
        if expected == "restart" {
            let summary = ferrydock::lifecycle::summarize(&operation_markers);
            assert_eq!(
                summary.len(),
                1,
                "A real Docker restart should display one annotation"
            );
            assert_eq!(summary[0].action, "restart");
            assert!(summary[0].details.contains("Docker event: kill"));
            assert!(summary[0].details.contains("exit code:"));
            restart_summary = Some(
                serde_json::json!({"visible_markers":summary.len(),"raw_events":operation_markers.len(),"text":summary[0].text(),"details":summary[0].details}),
            );
        }
        wait_for_container_state(&docker, &id, expected != "stop").await?;
    }
    if let Ok(directory) = env::var("FERRYDOCK_LIFECYCLE_EVIDENCE") {
        std::fs::write(
            std::path::Path::new(&directory).join("docker-events.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"status":"passed","container":id,"project":project,"events":observations,"stop_summary":stop_summary,"restart_summary":restart_summary,"external_docker_client":true}),
            )?,
        )?;
    }
    println!(
        "PASS production event listener observes external CLI stop/start/restart with Compose identity: {}",
        serde_json::to_string(&observations)?
    );
    Ok(())
}

#[tokio::test]
#[ignore = "Creates and operates only on a fixture supplied by scripts/verify-docker.sh"]
async fn docker_daily_loop_and_interactive_terminal() -> Result<()> {
    let id = env::var("FERRYDOCK_TEST_CONTAINER").context("Run scripts/verify-docker.sh")?;
    let connection = docker::resolve_connection().await?;
    let docker = docker::connect(&connection).await?;
    let fixture = docker.inspect_container(&id, None).await?;
    let labels = fixture
        .config
        .and_then(|config| config.labels)
        .unwrap_or_default();
    if labels.get("dev.ferrydock.fixture").map(String::as_str) != Some("true")
        || !fixture
            .name
            .as_deref()
            .unwrap_or("")
            .starts_with("/ferrydock-verify-")
    {
        bail!("Refusing operations on a container without the dedicated fixture identity");
    }
    let containers = docker::snapshot(&docker).await?;
    let probe = containers
        .iter()
        .find(|container| container.id == id)
        .context("Fixture not listed")?;
    assert_eq!(probe.service.as_deref(), Some("probe"));
    assert!(
        probe
            .project
            .as_deref()
            .unwrap_or("")
            .starts_with("ferrydock-verify-")
    );
    assert!(probe.running());

    let mut logs = docker.logs(
        &id,
        Some(
            LogsOptionsBuilder::default()
                .stdout(true)
                .stderr(true)
                .build(),
        ),
    );
    let mut log_text = String::new();
    while let Some(frame) = logs.next().await {
        log_text.push_str(&String::from_utf8_lossy(&frame?.into_bytes()));
    }
    assert!(log_text.contains("FERRY_FIXTURE_READY"));
    println!("PASS Compose discovery and real container logs");

    for action in [Action::Stop, Action::Start, Action::Restart] {
        docker::perform(&docker, &id, action).await?;
        wait_for_container_state(&docker, &id, !matches!(action, Action::Stop)).await?;
    }
    println!("PASS stop, start, restart and snapshot reconciliation");

    let (backend, mut events) = Backend::start();
    loop {
        match receive(&mut events).await? {
            Event::Connected(_) => break,
            Event::Disconnected(error) => bail!("Actor connection failed: {error}"),
            _ => {}
        }
    }
    let sample = time::timeout(Duration::from_secs(20), async {
        loop {
            if let Event::Metrics {
                id: sampled,
                result,
                ..
            } = receive(&mut events).await?
                && sampled == id
            {
                let sample = result.map_err(anyhow::Error::msg)?;
                if sample.cpu_percent.is_some() {
                    return Ok::<_, anyhow::Error>(sample);
                }
            }
        }
    })
    .await
    .context("No CPU baseline from the resource sampler")??;
    let limits = sample
        .limits
        .as_ref()
        .context("Resource limits were not inspected")?;
    assert_eq!(limits.memory, Some(64 * 1024 * 1024));
    assert_eq!(limits.cpu_cores, Some(0.5));
    assert_eq!(sample.memory_capacity, Some(64 * 1024 * 1024));
    assert!(
        sample
            .memory_used
            .is_some_and(|used| used > 0 && used < 64 * 1024 * 1024)
    );
    assert!(
        sample
            .cpu_percent
            .is_some_and(|cpu| cpu.is_finite() && cpu >= 0.0)
    );
    println!("PASS real worker CPU baseline, memory usage, 64 MiB limit and 0.5 CPU quota");

    let populate = tokio::process::Command::new("docker")
        .args([
            "exec",
            &id,
            "sh",
            "-c",
            "i=0; while [ $i -lt 600 ]; do echo FERRY_HISTORY_$i; i=$((i+1)); done > /proc/1/fd/1",
        ])
        .status()
        .await?;
    assert!(populate.success());
    backend.send(Command::Logs {
        id: id.clone(),
        generation: 40,
    });
    loop {
        match receive(&mut events).await? {
            Event::Log {
                generation: 40,
                data,
            } => {
                let text = String::from_utf8(data)?;
                assert_eq!(text.matches("FERRY_HISTORY_").count(), 500);
                assert!(text.contains("FERRY_HISTORY_100\n"));
                assert!(text.contains("FERRY_HISTORY_599\n"));
                break;
            }
            Event::LogsEnded {
                generation: 40,
                error,
            } => bail!("Initial history failed: {error:?}"),
            _ => {}
        }
    }
    let append = tokio::process::Command::new("docker")
        .args([
            "exec",
            &id,
            "sh",
            "-c",
            "echo FERRY_AFTER_HISTORY > /proc/1/fd/1",
        ])
        .status()
        .await?;
    assert!(append.success());
    loop {
        match receive(&mut events).await? {
            Event::Log {
                generation: 40,
                data,
            } => {
                let text = String::from_utf8(data)?;
                assert!(
                    !text.contains("FERRY_HISTORY_"),
                    "Snapshot records must not be replayed"
                );
                if text.contains("FERRY_AFTER_HISTORY") {
                    break;
                }
            }
            Event::LogsEnded {
                generation: 40,
                error,
            } => bail!("Live logs ended: {error:?}"),
            _ => {}
        }
    }
    println!("PASS initial 500-line history in one event and live resume without replay");
    let generation = 41;
    backend.send(Command::Terminal {
        id: id.clone(),
        generation,
        shell: Shell::Auto,
        columns: 80,
        rows: 24,
    });
    loop {
        match receive(&mut events).await? {
            Event::TerminalReady {
                generation: current,
            } if current == generation => break,
            Event::TerminalEnded {
                generation: current,
                error,
            } if current == generation => bail!("Could not open shell: {error:?}"),
            _ => {}
        }
    }
    let mut terminal = TerminalCore::new(100, 30);
    backend.send(Command::Resize {
        generation,
        columns: 100,
        rows: 30,
    });
    backend.send(Command::Input {
        generation,
        data: format!(
            "{}\r",
            r"stty size; printf '\033[31mFERRY_TERMINAL_OK\033[0m\n'"
        )
        .into_bytes(),
    });
    wait_for_text(&mut events, &mut terminal, generation, "FERRY_TERMINAL_OK").await?;
    assert!(
        terminal
            .visible_text()
            .lines()
            .any(|line| line.trim() == "30 100")
    );
    println!("PASS interactive shell, PTY resize and ANSI parsing");

    backend.send(Command::Input {
        generation,
        data: b"vi /tmp/ferrydock-terminal-smoke.txt\r".to_vec(),
    });
    time::sleep(Duration::from_millis(500)).await;
    backend.send(Command::Input {
        generation,
        data: b"ihello ferry".to_vec(),
    });
    time::sleep(Duration::from_millis(100)).await;
    backend.send(Command::Input {
        generation,
        data: vec![27],
    });
    time::sleep(Duration::from_millis(200)).await;
    backend.send(Command::Input {
        generation,
        data: b":wq\r".to_vec(),
    });
    time::sleep(Duration::from_millis(300)).await;
    backend.send(Command::Input {
        generation,
        data: format!(
            "{}\r",
            r"cat /tmp/ferrydock-terminal-smoke.txt; printf 'FERRY_EDITOR_DONE\n'"
        )
        .into_bytes(),
    });
    wait_for_text(&mut events, &mut terminal, generation, "FERRY_EDITOR_DONE").await?;
    assert!(terminal.visible_text().contains("hello ferry"));
    println!("PASS full-screen vi edit through the same terminal transport");

    backend.send(Command::Input {
        generation,
        data: b"exit\r".to_vec(),
    });
    loop {
        if let Event::TerminalEnded {
            generation: current,
            error,
        } = receive(&mut events).await?
            && current == generation
        {
            assert!(error.is_none());
            break;
        }
    }
    backend.send(Command::Terminal {
        id,
        generation: 42,
        shell: Shell::Bash,
        columns: 80,
        rows: 24,
    });
    loop {
        if let Event::TerminalEnded {
            generation: 42,
            error,
        } = receive(&mut events).await?
        {
            assert!(error.is_some());
            break;
        }
    }
    println!("PASS shell exit and missing-Bash error");
    Ok(())
}

#[tokio::test]
#[ignore = "Runs only in the disposable project owned by scripts/verify-docker.sh"]
async fn favorite_command_uses_an_interactive_pty_and_preserves_busy_session() -> Result<()> {
    use ferrydock::favorites::Target;
    let id = env::var("FERRYDOCK_TEST_CONTAINER").context("Run scripts/verify-docker.sh")?;
    let connection = docker::resolve_connection().await?;
    let docker = docker::connect(&connection).await?;
    let info = docker.inspect_container(&id, None).await?;
    let labels = info
        .config
        .and_then(|config| config.labels)
        .unwrap_or_default();
    let project = labels
        .get("com.docker.compose.project")
        .context("Missing project")?
        .clone();
    if labels.get("dev.ferrydock.fixture").map(String::as_str) != Some("true")
        || !project.starts_with("ferrydock-verify-")
    {
        bail!("Refusing a non-owned fixture");
    }
    let target = Target::Service {
        project,
        service: "probe".into(),
    };
    let (backend, mut events) = Backend::start();
    loop {
        match receive(&mut events).await? {
            Event::Connected(_) => break,
            Event::Disconnected(error) => bail!("Connection failed: {error}"),
            _ => {}
        }
    }
    let generation = 701;
    backend.send(Command::Terminal {
        id: id.clone(),
        generation,
        shell: match env::var("FERRYDOCK_FAVORITE_SHELL").as_deref() {
            Ok("bash") => Shell::Bash,
            Ok("auto") => Shell::Auto,
            _ => Shell::Sh,
        },
        columns: 96,
        rows: 28,
    });
    let mut terminal = TerminalCore::new(96, 28);
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    let run = |run, command: &str| Command::RunFavorite {
        id: id.clone(),
        generation,
        run,
        endpoint: connection.endpoint.clone(),
        target: target.clone(),
        command: command.into(),
    };
    backend.send(run(1, "stty size; printf 'FAVORITE_READY\\n'; read answer; printf '%s\\n' \"$answer\" > /tmp/ferrydock-favorite-proof; printf 'FAVORITE_DONE\\n'"));
    wait_for_text(&mut events, &mut terminal, generation, "FAVORITE_READY").await?;
    assert!(
        terminal
            .visible_text()
            .lines()
            .any(|line| line.trim() == "28 96")
    );
    backend.send(run(2, "touch /tmp/ferrydock-favorite-must-not-run"));
    assert!(
        wait_for_favorite(&mut events, &mut terminal, generation, 2)
            .await?
            .is_err()
    );
    backend.send(Command::Input {
        generation,
        data: b"favorite-interactive-proof\r".to_vec(),
    });
    assert_eq!(
        wait_for_favorite(&mut events, &mut terminal, generation, 1).await?,
        Ok(0)
    );
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    assert!(terminal.visible_text().contains("FAVORITE_DONE"));
    // Consecutive commands use the same shell, preserving cwd and exports.
    backend.send(run(
        3,
        "cd /tmp\nexport FERRY_FAVORITE_VALUE=kept\nprintf 'MULTILINE_DONE\\n'",
    ));
    assert_eq!(
        wait_for_favorite(&mut events, &mut terminal, generation, 3).await?,
        Ok(0)
    );
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    backend.send(run(4, "false"));
    assert_eq!(
        wait_for_favorite(&mut events, &mut terminal, generation, 4).await?,
        Ok(1)
    );
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    backend.send(run(
        5,
        "printf '%s:%s\\n' \"$PWD\" \"$FERRY_FAVORITE_VALUE\"",
    ));
    assert_eq!(
        wait_for_favorite(&mut events, &mut terminal, generation, 5).await?,
        Ok(0)
    );
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    assert!(
        terminal
            .visible_text()
            .lines()
            .any(|line| line.trim() == "/tmp:kept")
    );
    // A manually typed program and a partially edited line are also protected.
    backend.send(Command::Input {
        generation,
        data: b"sleep 1\r".to_vec(),
    });
    backend.send(run(6, "touch /tmp/ferrydock-favorite-must-not-run"));
    assert!(
        wait_for_favorite(&mut events, &mut terminal, generation, 6)
            .await?
            .is_err()
    );
    backend.send(Command::Input {
        generation,
        data: vec![3],
    });
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    backend.send(Command::Input {
        generation,
        data: b"unfinished".to_vec(),
    });
    backend.send(run(7, "touch /tmp/ferrydock-favorite-must-not-run"));
    assert!(
        wait_for_favorite(&mut events, &mut terminal, generation, 7)
            .await?
            .is_err()
    );
    backend.send(Command::Input {
        generation,
        data: vec![21, 3],
    });
    wait_for_prompt(&mut events, &mut terminal, generation)
        .await
        .with_context(|| {
            format!(
                "Prompt wait at line {}: {}",
                line!(),
                terminal.visible_text()
            )
        })?;
    let exec = docker.create_exec(&id, bollard::exec::CreateExecOptions { attach_stdout: Some(true), attach_stderr: Some(true), cmd: Some(vec!["sh", "-c", "cat /tmp/ferrydock-favorite-proof; test ! -e /tmp/ferrydock-favorite-must-not-run && echo BUSY_PRESERVED"]), ..Default::default() }).await?;
    let bollard::exec::StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await?
    else {
        bail!("Missing independent proof output");
    };
    let mut proof = Vec::new();
    while let Some(frame) = output.next().await {
        proof.extend(frame?.into_bytes());
    }
    let proof = String::from_utf8(proof)?;
    assert!(proof.contains("favorite-interactive-proof"));
    assert!(proof.contains("BUSY_PRESERVED"));
    if env::var_os("FERRYDOCK_EXPECT_PROMPT_HOOK").is_some() {
        backend.send(run(9, "test -s /tmp/ferrydock-existing-prompt-hook"));
        assert_eq!(
            wait_for_favorite(&mut events, &mut terminal, generation, 9).await?,
            Ok(0),
            "The existing PROMPT_COMMAND must still execute"
        );
        wait_for_prompt(&mut events, &mut terminal, generation).await?;
    }
    backend.send(run(8, "exit 7"));
    assert_eq!(
        wait_for_favorite(&mut events, &mut terminal, generation, 8).await?,
        Ok(7)
    );
    backend.send(Command::CloseTerminal);
    println!(
        "PASS favorite PTY 28x96; interactive input/file proof; same shell and prompt after success/failure; exit 0/1; multiline cwd/export persistence; busy/manual/partial-input rejection"
    );
    Ok(())
}

async fn wait_for_prompt(
    events: &mut mpsc::Receiver<Event>,
    terminal: &mut TerminalCore,
    generation: u64,
) -> Result<()> {
    time::timeout(Duration::from_secs(15), async {
        loop {
            match receive(events).await? {
                Event::TerminalPrompt {
                    generation: current,
                    at_prompt: true,
                    ..
                } if current == generation => return Ok(()),
                Event::TerminalData {
                    generation: current,
                    data,
                } if current == generation => terminal.feed(&data),
                Event::TerminalEnded {
                    generation: current,
                    error,
                } if current == generation => bail!("Shell ended before prompt: {error:?}"),
                _ => {}
            }
        }
    })
    .await
    .context("Timed out waiting for an empty shell prompt")?
}
async fn wait_for_favorite(
    events: &mut mpsc::Receiver<Event>,
    terminal: &mut TerminalCore,
    generation: u64,
    run: u64,
) -> Result<Result<i64, String>> {
    time::timeout(Duration::from_secs(15), async {
        loop {
            match receive(events).await? {
                Event::FavoriteFinished {
                    generation: current,
                    run: current_run,
                    result,
                } if current == generation && current_run == run => return Ok(result),
                Event::TerminalData {
                    generation: current,
                    data,
                } if current == generation => terminal.feed(&data),
                Event::TerminalEnded {
                    generation: current,
                    error,
                } if current == generation => {
                    bail!("Shell ended before favorite completed: {error:?}")
                }
                _ => {}
            }
        }
    })
    .await
    .context("Timed out waiting for favorite completion")?
}

#[tokio::test]
#[ignore = "requires an owned disposable Docker fixture"]
async fn quick_run_terminals_preserve_interactive_sessions() -> Result<()> {
    use ferrydock::favorites::Target;
    let id = env::var("FERRYDOCK_TEST_CONTAINER")?;
    let connection = docker::resolve_connection().await?;
    let docker = docker::connect(&connection).await?;
    let inspect = docker.inspect_container(&id, None).await?;
    let labels = inspect
        .config
        .and_then(|c| c.labels)
        .context("Fixture labels")?;
    let project = labels
        .get("com.docker.compose.project")
        .context("Fixture project")?
        .clone();
    if labels.get("dev.ferrydock.fixture").map(String::as_str) != Some("true")
        || !project.starts_with("ferrydock-verify-")
    {
        bail!("Refusing non-owned fixture");
    }
    let target = Target::Service {
        project,
        service: "probe".into(),
    };
    let (backend, mut events) = Backend::start();
    loop {
        match receive(&mut events).await? {
            Event::Connected(_) => break,
            Event::Disconnected(e) => bail!("{e}"),
            _ => {}
        }
    }
    let regular = 801;
    let quick1 = (1_u64 << 63) | 1;
    let quick2 = (1_u64 << 63) | 2;
    let mut primary = TerminalCore::new(96, 28);
    backend.send(Command::Terminal {
        id: id.clone(),
        generation: regular,
        shell: Shell::Sh,
        columns: 96,
        rows: 28,
    });
    wait_for_prompt(&mut events, &mut primary, regular).await?;
    backend.send(Command::Input{generation:regular,data:b"printf 'PRIMARY_WAITING\\n'; read answer; printf '%s' \"$answer\" > /tmp/ferrydock-quick-primary; printf 'PRIMARY_FINISHED\\n'\r".to_vec()});
    wait_for_text(&mut events, &mut primary, regular, "PRIMARY_WAITING").await?;
    let run = |generation, run, command: &str| Command::RunFavorite {
        id: id.clone(),
        generation,
        run,
        endpoint: connection.endpoint.clone(),
        target: target.clone(),
        command: command.into(),
    };
    backend.send(Command::QuickTerminal {
        id: id.clone(),
        generation: quick1,
    });
    let mut first = TerminalCore::new(100, 28);
    wait_for_prompt(&mut events, &mut first, quick1).await?;
    backend.send(run(quick1,1,"printf 'QUICK_WAITING\\n'; read answer; printf '%s' \"$answer\" > /tmp/ferrydock-quick-first"));
    wait_for_text(&mut events, &mut first, quick1, "QUICK_WAITING").await?;
    backend.send(Command::QuickTerminal {
        id: id.clone(),
        generation: quick2,
    });
    let mut second = TerminalCore::new(100, 28);
    wait_for_prompt(&mut events, &mut second, quick2).await?;
    backend.send(run(
        quick2,
        2,
        "printf second > /tmp/ferrydock-quick-second; printf 'QUICK_SECOND_DONE\\n'",
    ));
    assert_eq!(
        wait_for_favorite(&mut events, &mut second, quick2, 2).await?,
        Ok(0)
    );
    wait_for_prompt(&mut events, &mut second, quick2).await?;
    // Commands/input still route by generation after a second quick shell opens.
    backend.send(Command::Input {
        generation: quick1,
        data: b"first-answer\r".to_vec(),
    });
    assert_eq!(
        wait_for_favorite(&mut events, &mut first, quick1, 1).await?,
        Ok(0)
    );
    wait_for_prompt(&mut events, &mut first, quick1).await?;
    backend.send(Command::CloseQuickTerminal { generation: quick1 });
    backend.send(Command::Input {
        generation: regular,
        data: b"primary-answer\r".to_vec(),
    });
    wait_for_text(&mut events, &mut primary, regular, "PRIMARY_FINISHED").await?;
    // The remaining quick terminal survives closing its sibling.
    backend.send(run(quick2, 3, "false"));
    assert_eq!(
        wait_for_favorite(&mut events, &mut second, quick2, 3).await?,
        Ok(1)
    );
    wait_for_prompt(&mut events, &mut second, quick2).await?;
    let exec=docker.create_exec(&id,bollard::exec::CreateExecOptions{attach_stdout:Some(true),cmd:Some(vec!["sh","-c","test \"$(cat /tmp/ferrydock-quick-primary)\" = primary-answer && test \"$(cat /tmp/ferrydock-quick-first)\" = first-answer && test \"$(cat /tmp/ferrydock-quick-second)\" = second && printf INDEPENDENT_SESSIONS_OK"]),..Default::default()}).await?;
    let bollard::exec::StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await?
    else {
        bail!("Missing proof stream");
    };
    let mut proof = Vec::new();
    while let Some(frame) = output.next().await {
        proof.extend(frame?.into_bytes());
    }
    assert_eq!(String::from_utf8(proof)?, "INDEPENDENT_SESSIONS_OK");
    // Worker retention is bounded even if commands are sent without the UI.
    for n in 3..=7 {
        let generation = (1_u64 << 63) | n;
        backend.send(Command::QuickTerminal {
            id: id.clone(),
            generation,
        });
        let mut terminal = TerminalCore::new(100, 28);
        wait_for_prompt(&mut events, &mut terminal, generation).await?;
    }
    let excess = (1_u64 << 63) | 8;
    backend.send(Command::QuickTerminal {
        id: id.clone(),
        generation: excess,
    });
    loop {
        if let Event::TerminalEnded { generation, error } = receive(&mut events).await?
            && generation == excess
        {
            assert!(error.unwrap().contains("limit 6"));
            break;
        }
    }
    for n in 3..=7 {
        backend.send(Command::CloseQuickTerminal {
            generation: (1_u64 << 63) | n,
        });
    }
    backend.send(Command::CloseQuickTerminal { generation: quick2 });
    backend.send(Command::CloseTerminal);
    println!(
        "PASS independent regular + two quick PTYs; interactive inputs and files isolated; sibling close preserved; exit 0/1"
    );
    Ok(())
}
