//! Exercise the production Bollard request against an owned Unix API fixture.
//! No connection to the user's Docker daemon and no resource deletion.
#![cfg(unix)]

use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bollard::{API_DEFAULT_VERSION, Docker};
use ferrydock::volumes::{PruneMode, PruneResult, prune};

async fn check_prune(mode: PruneMode, response: &str, status: &str) -> anyhow::Result<PruneResult> {
    let path = std::env::temp_dir().join(format!(
        "ferrydock-volumes-prune-{}-{}.sock",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let response = response.to_owned();
    let status = status.to_owned();
    let server = std::thread::spawn(move || {
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        start.elapsed() < Duration::from_secs(5),
                        "Prune did not reach fixture"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("Fixture accept: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        let request = String::from_utf8(request).unwrap();
        let line = request.lines().next().unwrap();
        assert!(line.starts_with("POST /volumes/prune?"), "{line}");
        let all = if mode == PruneMode::AllUnused {
            "true"
        } else {
            "false"
        };
        assert!(
            line.contains(&format!("filters=%7B%22all%22%3A%5B%22{all}%22%5D%7D")),
            "Exact anonymous/all-unused filter: {line}"
        );
        write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
    });
    let docker = Docker::connect_with_unix(path.to_str().unwrap(), 5, API_DEFAULT_VERSION).unwrap();
    let result = prune(&docker, mode).await;
    let server_result = server.join();
    std::fs::remove_file(path).unwrap();
    server_result.unwrap();
    result
}

#[tokio::test]
async fn prune_modes_send_exact_filters_and_report_deleted_names_and_bytes() {
    for mode in [PruneMode::Anonymous, PruneMode::AllUnused] {
        let result = check_prune(
            mode,
            r#"{"VolumesDeleted":["fixture-volume"],"SpaceReclaimed":3145728}"#,
            "200 OK",
        )
        .await
        .unwrap();
        assert_eq!(result.deleted, vec!["fixture-volume"]);
        assert_eq!(result.reclaimed, Some(3 * 1_048_576));
        let result = check_prune(
            mode,
            r#"{"VolumesDeleted":null,"SpaceReclaimed":0}"#,
            "200 OK",
        )
        .await
        .unwrap();
        assert!(result.deleted.is_empty());
        assert_eq!(result.reclaimed, Some(0));
    }
}

#[tokio::test]
async fn prune_preserves_unavailable_space_errors_and_api_guard() {
    for response in [r#"{"VolumesDeleted":null}"#, r#"{"SpaceReclaimed":-1}"#] {
        assert_eq!(
            check_prune(PruneMode::Anonymous, response, "200 OK")
                .await
                .unwrap()
                .reclaimed,
            None
        );
    }
    let error = check_prune(
        PruneMode::AllUnused,
        r#"{"message":"a prune operation is already running"}"#,
        "409 Conflict",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("already running"));
    assert!(
        check_prune(PruneMode::AllUnused, "invalid", "200 OK")
            .await
            .is_err()
    );
    // Bollard checks that the socket exists at construction. No connection is
    // expected: an old API must be refused before anonymous could prune named data.
    let path = std::env::temp_dir().join(format!(
        "ferrydock-prune-old-api-{}.sock",
        std::process::id()
    ));
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let docker = Docker::connect_with_unix(
        path.to_str().unwrap(),
        5,
        &bollard::ClientVersion {
            major_version: 1,
            minor_version: 41,
        },
    )
    .unwrap();
    for mode in [PruneMode::Anonymous, PruneMode::AllUnused] {
        assert!(
            prune(&docker, mode)
                .await
                .unwrap_err()
                .to_string()
                .contains("1.42")
        );
    }
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(listener);
    std::fs::remove_file(path).unwrap();
}
