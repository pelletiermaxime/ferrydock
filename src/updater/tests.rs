use super::*;
use serde_json::json;

fn image(payload: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x7fELF\x02\x01\x01\0AI\x02\0".to_vec();
    bytes.extend_from_slice(payload);
    bytes
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn hash(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&SHA256, bytes).as_ref())
}
fn release(version: &str) -> serde_json::Value {
    let name = format!("ferrydock-{version}-linux-x86_64.AppImage");
    json!({"tag_name": format!("v{version}"), "draft": false, "prerelease": false,
    "assets": [
        {"name": name, "size": 100, "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}")},
        {"name": "SHA256SUMS", "size": 100, "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/v{version}/SHA256SUMS")}
    ]})
}
fn select(value: &serde_json::Value) -> Result<Option<Candidate>> {
    select_release(&serde_json::to_vec(value)?, "0.12.0")
}
fn installation(directory: &Path) -> Installation {
    let path = directory.join("FerryDock.AppImage");
    fs::write(&path, image(b"old version")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    Installation {
        identity: identity(&path).unwrap(),
        path,
    }
}
fn pending(directory: &Path, bytes: &[u8]) -> Pending {
    let mut file = tempfile::NamedTempFile::new_in(directory).unwrap();
    file.write_all(bytes).unwrap();
    Pending {
        file,
        size: bytes.len() as u64,
        version: "0.13.0".into(),
        checksum: ring::digest::digest(&SHA256, bytes).as_ref().to_vec(),
        workspace: None,
    }
}

#[test]
fn stable_release_selection_rejects_downgrades_incomplete_and_foreign_assets() {
    assert!(select(&release("0.11.0")).unwrap().is_none());
    assert!(select(&release("0.12.0")).unwrap().is_none());
    assert_eq!(
        select(&release("0.13.0")).unwrap().unwrap().version,
        "0.13.0"
    );
    for kind in ["draft", "prerelease"] {
        let mut r = release("0.13.0");
        r[kind] = json!(true);
        assert!(select(&r).is_err());
    }
    assert!(select(&release("0.13.0-beta.1")).is_err());
    let mut r = release("0.13.0");
    r["assets"][0]["browser_download_url"] = json!("https://example.com/payload");
    assert!(select(&r).is_err());
    let mut r = release("0.13.0");
    r["assets"][0]["size"] = json!(MAX_IMAGE + 1);
    assert!(select(&r).is_err());
    let mut r = release("0.13.0");
    r["assets"].as_array_mut().unwrap().remove(1);
    assert!(select(&r).is_err());
    let mut r = release("0.13.0");
    let duplicate = r["assets"][0].clone();
    r["assets"].as_array_mut().unwrap().push(duplicate);
    assert!(select(&r).is_err());
}

#[test]
fn checksum_requires_exact_unambiguous_filename_and_bounded_input() {
    let digest = hash(b"payload");
    let line = format!("{digest}  test.AppImage\n");
    assert_eq!(
        hex(&checksum(line.as_bytes(), "test.AppImage").unwrap()),
        digest
    );
    assert!(checksum(line.as_bytes(), "other.AppImage").is_err());
    assert!(checksum(format!("{line}{line}").as_bytes(), "test.AppImage").is_err());
    assert!(
        checksum(
            format!("{}  test.AppImage", "x".repeat(64)).as_bytes(),
            "test.AppImage"
        )
        .is_err()
    );
    assert!(
        checksum(
            format!("{digest} *test.AppImage").as_bytes(),
            "test.AppImage"
        )
        .is_ok()
    );
    assert!(bounded(&b"12345"[..], 4).is_err());
}

#[test]
fn preferences_persist_opt_out_and_throttle_successful_and_failed_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("updates.json");
    let mut prefs = Preferences::load(&path).unwrap();
    assert!(prefs.due(INTERVAL.as_secs()));
    prefs.last_check = 100;
    assert!(!prefs.due(101));
    assert!(prefs.due(100 + INTERVAL.as_secs()));
    assert!(prefs.due(99)); // Recover if the clock moves backwards.
    prefs.automatic = false;
    prefs.save(&path).unwrap();
    let saved = Preferences::load(&path).unwrap();
    assert!(!saved.due(u64::MAX));
    assert_eq!(saved.last_check, 100);
    fs::write(&path, b"invalid").unwrap();
    assert!(Preferences::load(&path).is_err());
}

// A real loopback HTTP boundary, with owned files only. Production URLs cannot
// be overridden by environment variables or user-supplied update metadata.
fn server(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<()>) {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        for bytes in responses {
            let started = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            started.elapsed() < Duration::from_secs(10),
                            "Fixture request timed out"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 4096);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            let _ = stream.write_all(&bytes);
        }
    });
    (base, handle)
}

#[test]
fn real_http_download_verification_and_atomic_apply_preserve_the_installed_path() {
    let dir = tempfile::tempdir().unwrap();
    let installed = installation(dir.path());
    let bytes = image(b"verified next version");
    let mut metadata = release("0.13.0");
    metadata["assets"][0]["size"] = json!(bytes.len());
    let sums = format!("{}  ferrydock-0.13.0-linux-x86_64.AppImage\n", hash(&bytes));
    let (url, http) = server(vec![
        serde_json::to_vec(&metadata).unwrap(),
        sums.into_bytes(),
        bytes.clone(),
    ]);
    let source = Source {
        client: Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        latest: url.clone(),
    };
    let mut candidate = select_release(&source.get(&url, MAX_RELEASE).unwrap(), "0.12.0")
        .unwrap()
        .unwrap();
    candidate.image.browser_download_url = url.clone();
    candidate.checksums.browser_download_url = url;
    let (events, _receiver) = mpsc::sync_channel(8);
    let update = source
        .download(candidate, &installed, &events, &AtomicBool::new(false))
        .unwrap();
    http.join().unwrap();
    assert_eq!(fs::read(&installed.path).unwrap(), image(b"old version"));
    install(update, &installed, |path| {
        assert_eq!(path, installed.path);
        assert_eq!(fs::read(path)?, bytes);
        assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o755);
        Ok(())
    })
    .unwrap();
    assert_eq!(fs::read(&installed.path).unwrap(), bytes);
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "Only the updated AppImage and its private staging directory should remain"
    );
}

#[test]
fn tampered_or_truncated_downloads_never_replace_the_application() {
    for corrupt in [image(b"tampered"), b"truncated".to_vec()] {
        let dir = tempfile::tempdir().unwrap();
        let installed = installation(dir.path());
        let mut candidate = select(&release("0.13.0")).unwrap().unwrap();
        candidate.image.size = corrupt.len() as u64;
        let sums = format!(
            "{}  {}\n",
            hash(&image(b"intended version")),
            candidate.image.name
        );
        let (url, http) = server(vec![sums.into_bytes(), corrupt]);
        candidate.checksums.browser_download_url = url.clone();
        candidate.image.browser_download_url = url;
        let source = Source {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            latest: String::new(),
        };
        let (events, _receiver) = mpsc::sync_channel(8);
        assert!(
            source
                .download(candidate, &installed, &events, &AtomicBool::new(false))
                .is_err()
        );
        http.join().unwrap();
        assert_eq!(fs::read(&installed.path).unwrap(), image(b"old version"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn reverify_before_install_rejects_modified_staging_and_changed_installation() {
    let dir = tempfile::tempdir().unwrap();
    let installed = installation(dir.path());
    let mut update = pending(dir.path(), &image(b"new version"));
    update.file.seek(SeekFrom::Start(12)).unwrap();
    update.file.write_all(b"bad version").unwrap();
    assert!(
        install(update, &installed, |_| panic!(
            "Must not launch corrupted update"
        ))
        .is_err()
    );
    assert_eq!(fs::read(&installed.path).unwrap(), image(b"old version"));
    let update = pending(dir.path(), &image(b"new version"));
    fs::write(&installed.path, image(b"externally replaced")).unwrap();
    assert!(
        install(update, &installed, |_| panic!(
            "Must not overwrite another update"
        ))
        .is_err()
    );
    assert_eq!(
        fs::read(&installed.path).unwrap(),
        image(b"externally replaced")
    );
}

#[test]
fn failed_relaunch_restores_the_previous_appimage_and_cleans_staging() {
    let dir = tempfile::tempdir().unwrap();
    let installed = installation(dir.path());
    let update = pending(dir.path(), &image(b"new version"));
    assert!(install(update, &installed, |_| bail!("Fixture launch failure")).is_err());
    assert_eq!(identity(&installed.path).unwrap(), installed.identity);
    assert_eq!(fs::read(&installed.path).unwrap(), image(b"old version"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn staging_lock_preserves_live_candidates_and_recovers_abandoned_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let installed = installation(dir.path());
    let workspace = Workspace::acquire(&installed).unwrap();
    let stale = workspace.path.join("candidate-abandoned.AppImage");
    fs::write(&stale, b"old partial download").unwrap();
    assert!(Workspace::acquire(&installed).is_err());
    assert!(
        stale.exists(),
        "Another instance must not remove a live candidate"
    );
    drop(workspace);
    let workspace = Workspace::acquire(&installed).unwrap();
    assert!(!stale.exists());
    assert_eq!(
        fs::read_dir(&workspace.path).unwrap().count(),
        1,
        "Retain only the lock file"
    );
}

#[test]
#[ignore = "downloads the latest public GitHub AppImage into owned verification files"]
fn public_github_release_download_and_checksum() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".verification/appimage-updater");
    fs::create_dir_all(&root).unwrap();
    let evidence = tempfile::Builder::new()
        .prefix("github-")
        .tempdir_in(root)
        .unwrap()
        .keep();
    let installed = installation(&evidence);
    let source = Source::github().unwrap();
    let candidate = select_release(&source.get(&source.latest, MAX_RELEASE).unwrap(), "0.0.0")
        .unwrap()
        .unwrap();
    let (events, _receiver) = mpsc::sync_channel(8);
    let pending = source
        .download(candidate, &installed, &events, &AtomicBool::new(false))
        .unwrap();
    fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(&json!({
            "status": "passed", "repository": REPOSITORY, "version": pending.version,
            "downloaded_bytes": pending.size, "verified_sha256": hex(&pending.checksum),
            "installed": false, "user_appimage_touched": false
        }))
        .unwrap(),
    )
    .unwrap();
    println!("Public GitHub download evidence: {}", evidence.display());
}

#[test]
#[ignore = "requires pinned cached AppImage tools; runs an owned extract-and-run fixture"]
fn packaged_appimage_replacement_relaunch_and_startup_failure_rollback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tools = root.join("target/appimage-tools");
    let builder = tools.join("appimagetool-x86_64.AppImage");
    let runtime = tools.join("runtime-x86_64");
    assert_eq!(
        hash(&fs::read(&builder).expect("Run Linux packaging once to cache AppImage tools")),
        "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
    );
    assert_eq!(
        hash(&fs::read(&runtime).unwrap()),
        "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"
    );
    let evidence_root = root.join(".verification/appimage-updater");
    fs::create_dir_all(&evidence_root).unwrap();
    let evidence = tempfile::Builder::new()
        .prefix("runtime-")
        .tempdir_in(&evidence_root)
        .unwrap()
        .keep();
    let extraction = evidence.join("tools");
    fs::create_dir(&extraction).unwrap();
    let output = Process::new(&builder)
        .arg("--appimage-extract")
        .current_dir(&extraction)
        .output()
        .unwrap();
    fs::write(evidence.join("extract.log"), &output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let appdir = evidence.join("Fixture.AppDir");
    fs::create_dir_all(appdir.join("usr/bin")).unwrap();
    fs::write(
        appdir.join("AppRun"),
        fs::read(root.join("packaging/linux/AppRun")).unwrap(),
    )
    .unwrap();
    fs::set_permissions(appdir.join("AppRun"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(appdir.join("fixture.desktop"), "[Desktop Entry]\nType=Application\nName=FerryDock updater fixture\nExec=ferrydock\nIcon=fixture\nCategories=Utility;\n").unwrap();
    fs::copy(
        root.join("packaging/linux/dev.ferrydock.FerryDock.svg"),
        appdir.join("fixture.svg"),
    )
    .unwrap();
    let build = |name: &str, script: &str| {
        fs::write(appdir.join("usr/bin/ferrydock"), script).unwrap();
        fs::set_permissions(
            appdir.join("usr/bin/ferrydock"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let path = evidence.join(name);
        let output = Process::new(extraction.join("squashfs-root/AppRun"))
            .arg("--runtime-file")
            .arg(&runtime)
            .arg(&appdir)
            .arg(&path)
            .env("ARCH", "x86_64")
            .output()
            .unwrap();
        fs::write(
            evidence.join(format!("{name}.log")),
            [&output.stdout[..], &output.stderr[..]].concat(),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    };
    let original = build("FerryDock fixture.AppImage", "#!/bin/sh\nexit 0\n");
    let new = build(
        "new.AppImage",
        "#!/bin/sh\nset -eu\nprintf '%s\\n' 'new-version' \"$APPIMAGE\" \"$APPDIR\" \"$LD_LIBRARY_PATH\" > \"$APPIMAGE.started\"\nsleep 2\nprintf done > \"$APPIMAGE.finished\"\n",
    );
    let broken = build("broken.AppImage", "#!/bin/sh\nexit 17\n");
    let installed = Installation {
        identity: identity(&original).unwrap(),
        path: original.clone(),
    };
    let update = pending(&evidence, &fs::read(&new).unwrap());
    install(update, &installed, relaunch).unwrap();
    let started = Instant::now();
    while !original
        .with_file_name("FerryDock fixture.AppImage.finished")
        .exists()
    {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Owned fixture did not finish"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let probe =
        fs::read_to_string(original.with_file_name("FerryDock fixture.AppImage.started")).unwrap();
    assert!(probe.starts_with(&format!("new-version\n{}\n", original.display())));
    let before_failure = hash(&fs::read(&original).unwrap());
    let installed = Installation {
        identity: identity(&original).unwrap(),
        path: original.clone(),
    };
    let bad_update = pending(&evidence, &fs::read(&broken).unwrap());
    assert!(install(bad_update, &installed, relaunch).is_err());
    assert_eq!(hash(&fs::read(&original).unwrap()), before_failure);
    fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(&json!({
            "status": "passed", "fixture_only": true, "ferrydock_desktop_launched": false,
            "path_with_spaces": original, "extract_and_run": true, "replacement_and_relaunch": true,
            "immediate_startup_failure_rollback": true, "owned_fixture_finished": true,
            "app_run_probe": probe, "updated_sha256": before_failure
        }))
        .unwrap(),
    )
    .unwrap();
    // Keep logs/results while discarding only this test's extracted build tool.
    fs::remove_dir_all(extraction).unwrap();
    println!("AppImage runtime evidence: {}", evidence.display());
}
