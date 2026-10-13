use anyhow::{Context, Result, bail};
use ferrydock::{
    compose::{self, Config, Settings},
    docker::{self, Backend, Command, Event},
};
use std::{env, path::PathBuf, time::Duration};
use tokio::{sync::mpsc, time};

async fn receive(events: &mut mpsc::Receiver<Event>) -> Result<Event> {
    time::timeout(Duration::from_secs(40), events.recv())
        .await
        .context("Compose worker event timed out")?
        .context("Worker exited")
}

async fn operation(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    project: &str,
    action: compose::Action,
    id: u64,
) -> Result<(Result<(), String>, String)> {
    backend.send(Command::Compose {
        project: project.into(),
        operation: id,
        action,
    });
    let mut output = String::new();
    loop {
        match receive(events).await? {
            Event::ComposeOutput {
                operation, data, ..
            } if operation == id => output.push_str(&String::from_utf8_lossy(&data)),
            Event::ComposeFinished {
                operation, result, ..
            } if operation == id => return Ok((result, output)),
            _ => {}
        }
    }
}

async fn rebuild_marker(endpoint: &str, container: &str) -> Result<String> {
    let output = time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("docker")
            .args([
                "--host",
                endpoint,
                "exec",
                container,
                "cat",
                "/ferry-rebuild-marker",
            ])
            .output(),
    )
    .await??;
    assert!(
        output.status.success(),
        "Cannot read fixture build marker: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

async fn wait_project_logs(
    events: &mut mpsc::Receiver<Event>,
    generation: u64,
    expected: &[(&str, Option<&str>, &str)],
    not_before: Option<chrono::DateTime<chrono::FixedOffset>>,
) -> Result<Vec<serde_json::Value>> {
    time::timeout(Duration::from_secs(25), async {
        let mut found = vec![false; expected.len()];
        let mut observed = Vec::new();
        loop {
            if let Event::ProjectLogBatch { generation: current, lines } = receive(events).await? && current == generation {
              for line in lines {
                if let Some(cutoff) = not_before {
                    let timestamp = line.text.split_whitespace().next().and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
                    if timestamp.is_none_or(|timestamp| timestamp < cutoff) { continue; }
                }
                for (index, (service, id, text)) in expected.iter().enumerate() {
                    if line.service == *service && id.is_none_or(|id| line.container == id) && line.text.contains(text) {
                        found[index] = true;
                        observed.push(serde_json::json!({"container":line.container,"service":line.service,"name":line.name,"text":line.text}));
                    }
                }
                if found.iter().all(|found| *found) { return Ok(observed); }
              }
            }
        }
    }).await.context("Project logs did not receive all expected sources")?
}

#[tokio::test]
#[ignore = "Operates only on the uniquely owned fixture from scripts/verify-compose.sh"]
async fn compose_project_controls_and_association() -> Result<()> {
    let project =
        env::var("FERRYDOCK_COMPOSE_TEST_PROJECT").context("Run scripts/verify-compose.sh")?;
    if !project.starts_with("ferrydock-compose-verify-") {
        bail!("Invalid fixture project identity");
    }
    let endpoint = env::var("FERRYDOCK_COMPOSE_TEST_ENDPOINT")?;
    let connection = docker::resolve_connection().await?;
    assert_eq!(connection.endpoint, endpoint);
    let docker = docker::connect(&connection).await?;
    let containers = docker::snapshot(&docker).await?;
    let owned = containers
        .iter()
        .filter(|container| container.project.as_deref() == Some(&project))
        .collect::<Vec<_>>();
    assert_eq!(
        owned.len(),
        2,
        "Fixture must contain the base and profiled services"
    );
    for container in &owned {
        let inspected = docker.inspect_container(&container.id, None).await?;
        let config = inspected.config.context("Fixture configuration")?;
        assert_eq!(
            config
                .labels
                .as_ref()
                .and_then(|labels| labels.get("dev.ferrydock.fixture"))
                .map(String::as_str),
            Some("true")
        );
        if container.service.as_deref() == Some("probe") {
            let environment = config.env.unwrap_or_default();
            assert!(environment.contains(&"FERRY_OVERRIDE=override".into()));
            assert!(
                environment
                    .contains(&"FERRY_COMPOSE_MARKER=FERRY_COMPOSE_ASSOCIATION_PROOF".into())
            );
        }
    }
    let directory = PathBuf::from(env::var("FERRYDOCK_COMPOSE_TEST_DIRECTORY")?);
    let config = Config {
        project: project.clone(),
        directory: directory.clone(),
        files: vec![
            directory.join("compose-project.yaml"),
            directory.join("compose-project.override.yaml"),
        ],
        env_files: vec![directory.join("compose-project.env")],
        profiles: vec!["extra".into()],
    };
    assert_eq!(owned[0].compose_files, config.files);
    let (backend, mut events) = Backend::start();
    loop {
        if let Event::ComposeProjects(projects) = receive(&mut events).await? {
            let found = projects
                .iter()
                .find(|item| item.name == project)
                .context("Project discovery")?;
            assert!(found.config.is_none());
            assert_eq!(found.hint.files, config.files);
            break;
        }
    }
    backend.send(Command::ProjectLogs {
        project: project.clone(),
        generation: 77,
    });
    let initial_logs = wait_project_logs(
        &mut events,
        77,
        &[
            ("probe", None, "FERRY_PROJECT_PROBE_TICK:"),
            ("extra", None, "FERRY_PROJECT_EXTRA_TICK:"),
        ],
        None,
    )
    .await
    .context("Initial project log sources")?;
    assert!(initial_logs.iter().all(|line| {
        owned
            .iter()
            .any(|container| line["container"].as_str() == Some(&container.id))
    }));
    let burst_started = std::time::Instant::now();
    for container in &owned {
        let exec = docker.create_exec(&container.id, bollard::exec::CreateExecOptions {
            cmd: Some(vec!["sh", "-c", "{ i=0; while [ \"$i\" -lt 5000 ]; do printf 'FERRY_LOG_STRESS:%04d\\n' \"$i\"; i=$((i+1)); done; echo FERRY_LOG_STRESS_DONE; } > /proc/1/fd/1"]),
            ..Default::default()
        }).await?;
        docker
            .start_exec(
                &exec.id,
                Some(bollard::exec::StartExecOptions {
                    detach: true,
                    tty: false,
                    ..Default::default()
                }),
            )
            .await?;
    }
    let (burst_lines, burst_batches) = time::timeout(Duration::from_secs(25), async {
        let mut lines_count = 0;
        let mut batches_count = 0;
        let mut completed = std::collections::HashSet::new();
        loop {
            if let Event::ProjectLogBatch {
                generation: 77,
                lines,
            } = receive(&mut events).await?
            {
                assert!(lines.len() <= ferrydock::project_logs::MAX_BATCH_LINES);
                let mut stress = false;
                for line in lines {
                    assert!(owned.iter().any(|container| container.id == line.container));
                    if line.text.contains("FERRY_LOG_STRESS:") {
                        lines_count += 1;
                        stress = true;
                    }
                    if line.text.contains("FERRY_LOG_STRESS_DONE") {
                        completed.insert(line.container);
                    }
                }
                if stress {
                    batches_count += 1;
                }
                if completed.len() == owned.len() {
                    return Ok::<_, anyhow::Error>((lines_count, batches_count));
                }
            }
        }
    })
    .await
    .context("Busy project log stream timed out")??;
    assert_eq!(
        burst_lines, 10_000,
        "Batching must preserve every line in the fixture burst"
    );
    assert!(
        burst_batches < burst_lines / 8,
        "Backlog must be coalesced, not delivered as individual UI messages"
    );
    let busy_logs = serde_json::json!({"lines":burst_lines,"batches":burst_batches,"wall_seconds":burst_started.elapsed().as_secs_f64()});
    let (result, _) =
        operation(&backend, &mut events, &project, compose::Action::Rebuild, 1).await?;
    assert!(
        result.is_err(),
        "Unassociated project operation must be refused"
    );
    backend.send(Command::ConfigureCompose {
        config: config.clone(),
    });
    loop {
        if let Event::ComposeConfigured {
            project: name,
            result,
        } = receive(&mut events).await?
            && name == project
        {
            result.map_err(anyhow::Error::msg)?;
            break;
        }
    }
    let restored = Settings::load(&Settings::path()?)?;
    assert_eq!(restored.config(&endpoint, &project), Some(&config));
    assert!(
        restored
            .config("unix:///another-daemon", &project)
            .is_none()
    );
    let mut transcripts = Vec::new();
    for (id, action) in [
        (2, compose::Action::Stop),
        (3, compose::Action::Start),
        (4, compose::Action::Restart),
    ] {
        let before = docker
            .inspect_container(&owned[0].id, None)
            .await?
            .state
            .and_then(|state| state.started_at);
        let (result, output) = operation(&backend, &mut events, &project, action, id).await?;
        result.map_err(anyhow::Error::msg)?;
        assert!(!output.is_empty(), "Real CLI output must be streamed");
        for container in &owned {
            let inspected = docker.inspect_container(&container.id, None).await?;
            let state = inspected.state.context("Fixture state")?;
            assert_eq!(state.running, Some(action != compose::Action::Stop));
            if action == compose::Action::Restart && container.id == owned[0].id {
                assert_ne!(state.started_at, before);
            }
        }
        transcripts.push(serde_json::json!({"action": action.label(), "output": output}));
    }
    let mut restarted_after = None;
    for container in &owned {
        let started = docker
            .inspect_container(&container.id, None)
            .await?
            .state
            .and_then(|state| state.started_at)
            .context("Restarted container timestamp")?;
        let started = chrono::DateTime::parse_from_rfc3339(&started)?;
        restarted_after = Some(restarted_after.map_or(
            started,
            |previous: chrono::DateTime<chrono::FixedOffset>| previous.max(started),
        ));
    }
    let restarted_logs = wait_project_logs(
        &mut events,
        77,
        &[
            ("probe", None, "FERRY_PROJECT_PROBE_TICK:"),
            ("extra", None, "FERRY_PROJECT_EXTRA_TICK:"),
        ],
        restarted_after,
    )
    .await
    .context("Project log sources after restart")?;
    let old_extra = owned
        .iter()
        .find(|container| container.service.as_deref() == Some("extra"))
        .context("Extra fixture service")?;
    let recreated = time::timeout(
        Duration::from_secs(25),
        tokio::process::Command::new("docker")
            .args(config.args(&endpoint, compose::Action::Start))
            .args(["--force-recreate", "extra"])
            .output(),
    )
    .await??;
    assert!(
        recreated.status.success(),
        "Fixture recreation failed: {}",
        String::from_utf8_lossy(&recreated.stderr)
    );
    let refreshed = docker::snapshot(&docker).await?;
    let new_extra = refreshed
        .iter()
        .find(|container| {
            container.project.as_deref() == Some(&project)
                && container.service.as_deref() == Some("extra")
        })
        .context("Recreated service")?;
    assert_ne!(new_extra.id, old_extra.id);
    let recreated_logs = wait_project_logs(
        &mut events,
        77,
        &[("extra", Some(&new_extra.id), "FERRY_PROJECT_EXTRA_BOOT")],
        None,
    )
    .await
    .context("Project log source after fixture recreation")?;
    let old_probe = owned
        .iter()
        .find(|container| container.service.as_deref() == Some("probe"))
        .context("Buildable fixture service")?;
    let original_image = docker.inspect_container(&old_probe.id, None).await?.image;
    assert_eq!(
        rebuild_marker(&endpoint, &old_probe.id).await?,
        "before-rebuild"
    );
    std::fs::write(directory.join("rebuild-marker"), "after-rebuild\n")?;
    let (result, output) =
        operation(&backend, &mut events, &project, compose::Action::Rebuild, 6).await?;
    result.map_err(anyhow::Error::msg)?;
    assert!(
        !output.is_empty(),
        "Build and startup output must be streamed"
    );
    let rebuilt = docker::snapshot(&docker).await?;
    let rebuilt_probe = rebuilt
        .iter()
        .find(|container| {
            container.project.as_deref() == Some(&project)
                && container.service.as_deref() == Some("probe")
        })
        .context("Rebuilt fixture service")?;
    assert_ne!(
        rebuilt_probe.id, old_probe.id,
        "Rebuild must recreate the buildable service"
    );
    let rebuilt_image = docker
        .inspect_container(&rebuilt_probe.id, None)
        .await?
        .image;
    assert_ne!(
        rebuilt_image, original_image,
        "Changed build inputs must produce a new image"
    );
    assert_eq!(
        rebuild_marker(&endpoint, &rebuilt_probe.id).await?,
        "after-rebuild"
    );
    let rebuilt_extra = rebuilt
        .iter()
        .find(|container| {
            container.project.as_deref() == Some(&project)
                && container.service.as_deref() == Some("extra")
        })
        .context("Recreated profiled service")?;
    assert_ne!(
        rebuilt_extra.id, new_extra.id,
        "Rebuild must restart configured services without a build too"
    );
    assert!(rebuilt_probe.running() && rebuilt_extra.running());
    let rebuild_logs = wait_project_logs(
        &mut events,
        77,
        &[
            (
                "probe",
                Some(&rebuilt_probe.id),
                "FERRY_PROJECT_PROBE_TICK:",
            ),
            (
                "extra",
                Some(&rebuilt_extra.id),
                "FERRY_PROJECT_EXTRA_TICK:",
            ),
        ],
        None,
    )
    .await
    .context("Project log sources after rebuild")?;
    transcripts.push(
        serde_json::json!({"action":compose::Action::Rebuild.label(),"output":output,
        "before_container":old_probe.id,"after_container":rebuilt_probe.id,
        "before_image":original_image,"after_image":rebuilt_image,"marker":"after-rebuild"}),
    );

    // A failed build must report its output and leave the running services intact.
    let dockerfile = directory.join("compose-project.Dockerfile");
    let valid_dockerfile = std::fs::read(&dockerfile)?;
    std::fs::write(
        &dockerfile,
        "FROM alpine:3.23\nRUN echo FERRY_EXPECTED_BUILD_FAILURE >&2; exit 1\n",
    )?;
    let (result, output) =
        operation(&backend, &mut events, &project, compose::Action::Rebuild, 7).await?;
    assert!(result.is_err(), "Broken Dockerfile must fail the rebuild");
    assert!(
        output.contains("FERRY_EXPECTED_BUILD_FAILURE"),
        "Build error must be streamed: {output}"
    );
    for container in [rebuilt_probe, rebuilt_extra] {
        let inspected = docker.inspect_container(&container.id, None).await?;
        assert_eq!(
            inspected.state.and_then(|state| state.running),
            Some(true),
            "Failed build must preserve the running services"
        );
    }
    assert_eq!(
        rebuild_marker(&endpoint, &rebuilt_probe.id).await?,
        "after-rebuild"
    );
    transcripts.push(
        serde_json::json!({"action":"Failed rebuild","error":result.unwrap_err(),"output":output}),
    );
    std::fs::write(dockerfile, valid_dockerfile)?;
    backend.send(Command::CloseProjectLogs);
    let evidence = PathBuf::from(env::var("FERRYDOCK_COMPOSE_TEST_EVIDENCE")?);
    let invalid_file = evidence.join("invalid-compose.yaml");
    std::fs::write(&invalid_file, "services: [\n")?;
    backend.send(Command::ConfigureCompose {
        config: Config {
            files: vec![invalid_file],
            ..config.clone()
        },
    });
    loop {
        if let Event::ComposeConfigured {
            project: name,
            result,
        } = receive(&mut events).await?
            && name == project
        {
            result.map_err(anyhow::Error::msg)?;
            break;
        }
    }
    let (result, output) =
        operation(&backend, &mut events, &project, compose::Action::Start, 5).await?;
    assert!(
        result.is_err(),
        "Invalid Compose YAML must return a process failure"
    );
    assert!(
        !output.is_empty(),
        "CLI failure output must reach the worker event boundary"
    );
    transcripts.push(serde_json::json!({"action":"Invalid configuration", "error":result.unwrap_err(), "output":output}));
    backend.send(Command::ConfigureCompose {
        config: config.clone(),
    });
    loop {
        if let Event::ComposeConfigured {
            project: name,
            result,
        } = receive(&mut events).await?
            && name == project
        {
            result.map_err(anyhow::Error::msg)?;
            break;
        }
    }
    // Removing a file after association must surface a failure without launching
    // a CLI against a guessed/default project configuration.
    let missing = Config {
        files: vec![directory.join("not-present.yaml")],
        ..config.clone()
    };
    backend.send(Command::ConfigureCompose { config: missing });
    loop {
        if let Event::ComposeConfigured {
            project: name,
            result,
        } = receive(&mut events).await?
            && name == project
        {
            assert!(result.is_err());
            break;
        }
    }
    assert_eq!(
        Settings::load(&Settings::path()?)?.config(&endpoint, &project),
        Some(&config)
    );
    drop(backend);
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"passed", "project":project,"endpoint":endpoint,"configuration":config,"operations":transcripts,"project_logs":{"initial":initial_logs,"after_restart":restarted_logs,"recreated":recreated_logs,"after_rebuild":rebuild_logs,"busy":busy_logs},"checks":["label discovery","association required","persistent endpoint-scoped configuration","ordered override and env file","profiled service","Stop Start Restart side effects","changed build input produces new image and container with updated contents","rebuild restarts profiled image-only service","project logs reattach after rebuild","failed build streams errors and preserves running services","streamed CLI output","ambient Compose overrides ignored","real CLI failure output and nonzero exit","invalid association preserves existing settings","combined project stdout and stderr with source identity","live logs after restart","recreated service stream discovery without reopening","10000-line busy project burst coalesced without lost lines"]}),
        )?,
    )?;
    println!(
        "PASS Compose discovery, saved association, Stop/Start/Restart/Rebuild, build failure, output and scoped configuration"
    );
    Ok(())
}
