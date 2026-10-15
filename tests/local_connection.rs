//! Production API negotiation through an owned Unix socket or Windows pipe.
mod support;

#[tokio::test]
async fn selected_local_endpoint_negotiates_its_api_version() {
    let mut fixture = support::LocalApi::new(
        "GET /version HTTP/1.1".into(),
        "200 OK",
        r#"{"ApiVersion":"1.42"}"#,
    );
    let docker = ferrydock::docker::connect(&ferrydock::docker::Connection {
        name: "owned local API".into(),
        endpoint: fixture.endpoint.clone(),
    })
    .await
    .unwrap();
    assert_eq!(docker.client_version().minor_version, 42);
    fixture.finish().await;
}
