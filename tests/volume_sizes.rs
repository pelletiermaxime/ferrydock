//! Exercise the production Bollard request against an owned Unix API fixture.
//! No connection to the user's Docker daemon and no resource mutation.
#![cfg(unix)]

use std::{
    io::{Read, Write},
    os::unix::net::UnixListener,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bollard::ClientVersion;
use ferrydock::volumes::{VolumeSize, gather_sizes};

async fn check_sizes(
    response: &str,
    status: &str,
    minor_version: usize,
) -> anyhow::Result<Vec<VolumeSize>> {
    let path = std::env::temp_dir().join(format!(
        "ferrydock-sizes-{}-{}.sock",
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
                        "Size request did not reach fixture"
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
        assert_eq!(
            line,
            format!("GET /v1.{minor_version}/system/df?type=volume&verbose=true HTTP/1.1")
        );
        write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
    });
    let result = gather_sizes(
        path.to_str().unwrap(),
        &ClientVersion {
            major_version: 1,
            minor_version,
        },
    )
    .await;
    let server_result = server.join();
    std::fs::remove_file(path).unwrap();
    server_result.unwrap();
    result
}

#[tokio::test]
async fn filtered_usage_supports_legacy_and_modern_docker_responses() {
    let items = r#"[{"Name":"data","Driver":"local","Mountpoint":"/data","CreatedAt":"2026-10-13T00:00:00Z","UsageData":{"Size":10485760}},{"Name":"empty","Driver":"local","Mountpoint":"/empty","UsageData":{"Size":0}},{"Name":"remote","Driver":"nfs","Mountpoint":"/remote","UsageData":{"Size":-1}},{"Name":"missing","Driver":"local","Mountpoint":"/missing"}]"#;
    for (minor_version, response) in [
        (53, format!(r#"{{"Volumes":{items}}}"#)),
        (
            56,
            format!(r#"{{"VolumeUsage":{{"Items":{items},"TotalCount":4}}}}"#),
        ),
    ] {
        let sizes = check_sizes(&response, "200 OK", minor_version)
            .await
            .unwrap();
        assert_eq!(
            sizes.iter().map(|v| v.size).collect::<Vec<_>>(),
            vec![Some(10 * 1_048_576), Some(0), None, None]
        );
        assert_eq!(sizes[0].created.as_deref(), Some("2026-10-13T00:00:00Z"));
    }
}

#[tokio::test]
async fn empty_usage_errors_and_unsupported_api_remain_explicit() {
    for response in [
        r#"{"Volumes":null}"#,
        r#"{"Volumes":[]}"#,
        r#"{"VolumeUsage":{"TotalCount":0,"Items":null}}"#,
    ] {
        assert!(
            check_sizes(response, "200 OK", 53)
                .await
                .unwrap()
                .is_empty()
        );
    }
    for response in ["invalid", "{}", r#"{"VolumeUsage":{"TotalCount":2}}"#] {
        assert!(check_sizes(response, "200 OK", 53).await.is_err());
    }
    assert!(
        check_sizes(
            r#"{"message":"a disk usage operation is already running"}"#,
            "409 Conflict",
            53
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("already running")
    );
    let error = gather_sizes(
        "/no-connection-expected.sock",
        &ClientVersion {
            major_version: 1,
            minor_version: 41,
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("1.42"));
}
