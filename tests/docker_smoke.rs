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
