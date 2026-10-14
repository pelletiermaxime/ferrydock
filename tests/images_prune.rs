//! Exercise the production Bollard request against an owned Unix API fixture.
//! No connection to the user's Docker daemon and no image deletion.
#![cfg(unix)]

use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bollard::{API_DEFAULT_VERSION, Docker};
use ferrydock::images::{PruneMode, prune};

async fn check_prune(mode: PruneMode, response: &str, status: &str) -> anyhow::Result<Option<u64>> {
    let path = std::env::temp_dir().join(format!(
        "ferrydock-prune-{}-{}.sock",
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
        assert!(line.starts_with("POST /"), "{line}");
        let value = if mode == PruneMode::Untagged {
            "true"
        } else {
            "false"
        };
        assert!(
            line.contains(&format!(
                "/images/prune?filters=%7B%22dangling%22%3A%5B%22{value}%22%5D%7D"
            )),
            "{line}"
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
async fn prune_modes_send_docker_filters_and_report_actual_reclaimed_bytes() {
    for mode in [PruneMode::Untagged, PruneMode::AllUnused] {
        assert_eq!(check_prune(mode, r#"{"ImagesDeleted":[{"Untagged":"app:old"},{"Deleted":"sha256:layer"}],"SpaceReclaimed":3145728}"#, "200 OK").await.unwrap(), Some(3 * 1_048_576));
    }
}

#[tokio::test]
async fn prune_preserves_unavailable_space_and_docker_errors() {
    for response in [r#"{"ImagesDeleted":null}"#, r#"{"SpaceReclaimed":-1}"#] {
        assert_eq!(
            check_prune(PruneMode::Untagged, response, "200 OK")
                .await
                .unwrap(),
            None
        );
    }
    let error = check_prune(
        PruneMode::AllUnused,
        r#"{"message":"prune already running"}"#,
        "409 Conflict",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("prune already running"));
}
