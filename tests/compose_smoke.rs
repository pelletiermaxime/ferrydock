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

async fn fixture_cli(endpoint: &str, args: &[&str]) -> Result<String> {
    let output = time::timeout(
        Duration::from_secs(45),
        tokio::process::Command::new("docker")
            .args(["--host", endpoint])
            .args(args)
            .output(),
    )
    .await??;
    if !output.status.success() {
        bail!(
            "Fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

async fn save_association(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    config: &Config,
) -> Result<()> {
    backend.send(Command::ConfigureCompose {
        config: config.clone(),
    });
    loop {
        if let Event::ComposeConfigured { project, result } = receive(events).await?
            && project == config.project
        {
            return result.map(|_| ()).map_err(anyhow::Error::msg);
        }
    }
}

async fn verify_cleanup(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    config: &Config,
    endpoint: &str,
) -> Result<serde_json::Value> {
    let project = &config.project;
    let directory = &config.directory;
    let external_volume = format!("{project}-external-volume");
    let external_network = format!("{project}-external-network");
    let ownership = format!("dev.ferrydock.fixture.project={project}");
    fixture_cli(
        endpoint,
        &[
            "volume",
            "create",
            "--label",
            "dev.ferrydock.fixture=true",
            "--label",
            &ownership,
            &external_volume,
        ],
    )
    .await?;
    fixture_cli(
        endpoint,
        &[
            "network",
            "create",
            "--label",
            "dev.ferrydock.fixture=true",
            "--label",
            &ownership,
            &external_network,
        ],
    )
    .await?;
    let bind = directory.join("cleanup-bind");
    std::fs::create_dir(&bind)?;
    std::fs::write(bind.join("marker"), "bind data must survive")?;
    let mut cleanup = config.clone();
    cleanup
        .files
        .push(directory.join("compose-project.cleanup.yaml"));
    // Setup enables the fixture's profiled local-build service. The saved
    // configuration omits that profile so cleanup must cover all profiles.
    let mut setup = cleanup.clone();
    setup.profiles.push("cleanup".into());
    let setup_output = time::timeout(
        Duration::from_secs(45),
        tokio::process::Command::new("docker")
            .args(setup.args(endpoint, compose::Action::Rebuild))
            .output(),
    )
    .await??;
    assert!(
        setup_output.status.success(),
        "{}",
        String::from_utf8_lossy(&setup_output.stderr)
    );
    let local_image = format!("{project}-cleanup-local");
    let named_volume = format!("{project}_cleanup-data");
    save_association(backend, events, &cleanup).await?;
    let (result, kept_output) = operation(
        backend,
        events,
        project,
        compose::Action::Cleanup {
            volumes: false,
            images: false,
        },
        80,
    )
    .await?;
    result.map_err(anyhow::Error::msg)?;
    fixture_cli(endpoint, &["volume", "inspect", &named_volume]).await?;
    fixture_cli(endpoint, &["image", "inspect", &local_image]).await?;
    assert!(
        Settings::load(&Settings::path()?)?
            .config(endpoint, project)
            .is_none()
    );

    let anonymous_file = directory.join("cleanup-anonymous.yaml");
    std::fs::write(
        &anonymous_file,
        "services:\n  cleanup-local:\n    volumes:\n      - /scratch\n",
    )?;
    cleanup.files.push(anonymous_file);
    setup = cleanup.clone();
    setup.profiles.push("cleanup".into());
    let output = time::timeout(
        Duration::from_secs(45),
        tokio::process::Command::new("docker")
            .args(setup.args(endpoint, compose::Action::Start))
            .output(),
    )
    .await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let container_filter = format!("label=com.docker.compose.project={project}");
    let local_container = fixture_cli(
        endpoint,
        &[
            "ps",
            "-q",
            "--filter",
            &container_filter,
            "--filter",
            "label=com.docker.compose.service=cleanup-local",
        ],
    )
    .await?;
    let mounts: serde_json::Value = serde_json::from_str(
        &fixture_cli(
            endpoint,
            &["inspect", "--format", "{{json .Mounts}}", &local_container],
        )
        .await?,
    )?;
    let anonymous = mounts
        .as_array()
        .unwrap()
        .iter()
        .find(|mount| mount["Destination"] == "/scratch")
        .and_then(|mount| mount["Name"].as_str())
        .context("Anonymous fixture volume")?
        .to_owned();
    // Record the exact anonymous mount of this owned fixture for interrupted-run cleanup.
    std::fs::write(directory.join("cleanup-anonymous-volume"), &anonymous)?;
    let orphan = format!("{project}-cleanup-orphan");
    let project_label = format!("com.docker.compose.project={project}");
    fixture_cli(
        endpoint,
        &[
            "run",
            "-d",
            "--name",
            &orphan,
            "--label",
            &project_label,
            "--label",
            "com.docker.compose.service=removed-service",
            "--label",
            "com.docker.compose.config-hash=fixture",
            "--label",
            "dev.ferrydock.fixture=true",
            "alpine:3.23",
            "sleep",
            "3600",
        ],
    )
    .await?;
    save_association(backend, events, &cleanup).await?;
    let (result, removed_output) = operation(
        backend,
        events,
        project,
        compose::Action::Cleanup {
            volumes: true,
            images: true,
        },
        81,
    )
    .await?;
    result.map_err(anyhow::Error::msg)?;
    assert!(
        fixture_cli(endpoint, &["ps", "-aq", "--filter", &container_filter])
            .await?
            .is_empty()
    );
    assert!(
        fixture_cli(
            endpoint,
            &[
                "volume",
                "ls",
                "-q",
                "--filter",
                &format!("name=^{named_volume}$")
            ]
        )
        .await?
        .is_empty()
    );
    assert!(
        fixture_cli(
            endpoint,
            &[
                "volume",
                "ls",
                "-q",
                "--filter",
                &format!("name=^{anonymous}$")
            ]
        )
        .await?
        .is_empty()
    );
    assert!(
        fixture_cli(endpoint, &["image", "ls", "-q", &local_image])
            .await?
            .is_empty()
    );
    assert!(
        fixture_cli(
            endpoint,
            &[
                "network",
                "ls",
                "-q",
                "--filter",
                &format!("name=^{project}_default$")
            ]
        )
        .await?
        .is_empty()
    );
    fixture_cli(endpoint, &["volume", "inspect", &external_volume]).await?;
    fixture_cli(endpoint, &["network", "inspect", &external_network]).await?;
    let tagged_image = format!("ferrydock-compose-verify:{project}");
    fixture_cli(endpoint, &["image", "inspect", &tagged_image]).await?;
    fixture_cli(endpoint, &["image", "inspect", "alpine:3.23"]).await?;
    assert_eq!(
        std::fs::read_to_string(bind.join("marker"))?,
        "bind data must survive"
    );
    assert!(
        Settings::load(&Settings::path()?)?
            .config(endpoint, project)
            .is_none()
    );
    loop {
        if let Event::ComposeProjects(projects) = receive(events).await?
            && !projects.iter().any(|item| &item.name == project)
        {
            break;
        }
    }
    Ok(
        serde_json::json!({"kept_volumes_and_images":true,"removed_containers_and_orphans":true,"removed_named_and_anonymous_volumes":true,"removed_local_generated_image":true,"removed_project_network":true,"preserved_external_resources_tagged_images_and_bind_data":true,"removed_saved_association_and_project":true,"keep_output":kept_output,"remove_output":removed_output}),
    )
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
    let mut owned = containers
        .iter()
        .filter(|container| container.project.as_deref() == Some(&project))
        .cloned()
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
            let automatic = found.config.as_ref().context("Automatic association")?;
            assert_eq!(automatic.files, config.files);
            assert_eq!(automatic.env_files, config.env_files);
            assert!(automatic.profiles.is_empty());
            assert_eq!(found.worktree, Some(directory.canonicalize()?));
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
    let probe = owned
        .iter()
        .find(|container| container.service.as_deref() == Some("probe"))
        .context("Probe fixture")?;
    let before_auto_restart = docker
        .inspect_container(&probe.id, None)
        .await?
        .state
        .and_then(|state| state.started_at);
    let (result, _) =
        operation(&backend, &mut events, &project, compose::Action::Restart, 1).await?;
    result
        .map_err(anyhow::Error::msg)
        .context("Automatically associated Restart")?;
    let after_auto_restart = docker
        .inspect_container(&probe.id, None)
        .await?
        .state
        .and_then(|state| state.started_at);
    assert_ne!(
        before_auto_restart, after_auto_restart,
        "Auto-associated action must reach Docker"
    );
    assert!(
        Settings::load(&Settings::path()?)?
            .config(&endpoint, &project)
            .is_none(),
        "Discovery must not replace explicit saved settings"
    );
    let (result, _) = operation(
        &backend,
        &mut events,
        "ferrydock-unknown-project",
        compose::Action::Rebuild,
        100,
    )
    .await?;
    assert!(result.is_err(), "Unknown project operation must be refused");
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
        if action == compose::Action::Start {
            // Remove one stopped fixture member to prove Start restores a
            // missing service and subsequent checks follow its replacement.
            docker.remove_container(&owned[0].id, None).await?;
        }
        let (result, output) = operation(&backend, &mut events, &project, action, id).await?;
        result.map_err(anyhow::Error::msg)?;
        assert!(!output.is_empty(), "Real CLI output must be streamed");
        // `up` may recreate containers when Compose detects configuration or
        // image metadata changes. Check its current members, not stale IDs.
        if action == compose::Action::Start {
            owned = docker::snapshot(&docker)
                .await?
                .into_iter()
                .filter(|container| container.project.as_deref() == Some(&project))
                .collect();
            assert_eq!(owned.len(), 2, "Start must restore both fixture services");
            for service in ["probe", "extra"] {
                assert!(
                    owned
                        .iter()
                        .any(|container| { container.service.as_deref() == Some(service) }),
                    "Start must restore the {service} service"
                );
            }
        }
        for container in &owned {
            let inspected = docker
                .inspect_container(&container.id, None)
                .await
                .with_context(|| format!("Inspect {} after {}", container.name, action.label()))?;
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
    let cleanup_result = verify_cleanup(&backend, &mut events, &config, &endpoint).await?;
    drop(backend);
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"passed", "project":project,"endpoint":endpoint,"configuration":config,"operations":transcripts,"cleanup":cleanup_result,"project_logs":{"initial":initial_logs,"after_restart":restarted_logs,"recreated":recreated_logs,"after_rebuild":rebuild_logs,"busy":busy_logs},"checks":["label discovery","linked Git worktree discovery","automatic ordered-file and environment association","automatic Restart changes container start time without saving settings","unknown project refused","persistent endpoint-scoped configuration","ordered override and env file","profiled service","Stop Start Restart side effects","changed build input produces new image and container with updated contents","rebuild restarts profiled image-only service","project logs reattach after rebuild","failed build streams errors and preserves running services","streamed CLI output","ambient Compose overrides ignored","real CLI failure output and nonzero exit","invalid association preserves existing settings","combined project stdout and stderr with source identity","live logs after restart","recreated service stream discovery without reopening","10000-line busy project burst coalesced without lost lines"]}),
        )?,
    )?;
    println!(
        "PASS Compose source/worktree discovery, automatic and saved association, Stop/Start/Restart/Rebuild, build failure, output and scoped configuration"
    );
    Ok(())
}
