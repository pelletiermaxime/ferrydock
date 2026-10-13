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
