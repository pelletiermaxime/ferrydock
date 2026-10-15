//! Exercise production volume-size requests against an owned local API fixture.
//! Uses Unix sockets on Unix and named pipes on Windows; never a real daemon.
mod support;

use bollard::ClientVersion;
use ferrydock::volumes::{VolumeSize, gather_sizes};

async fn check_sizes(
    response: &str,
    status: &str,
    minor_version: usize,
) -> anyhow::Result<Vec<VolumeSize>> {
    let mut fixture = support::LocalApi::new(
        format!("GET /v1.{minor_version}/system/df?type=volume&verbose=true HTTP/1.1"),
        status,
        response,
    );
    let result = gather_sizes(
        &fixture.endpoint,
        &ClientVersion {
            major_version: 1,
            minor_version,
        },
    )
    .await;
    fixture.finish().await;
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
