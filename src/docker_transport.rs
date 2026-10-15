//! Local Docker transports shared by Bollard and the filtered disk-usage request.
use anyhow::{Result, bail};
use bollard::{API_DEFAULT_VERSION, Docker};
use hyper::{Response, body::Incoming};

pub(crate) fn client(endpoint: &str) -> Result<Docker> {
    #[cfg(unix)]
    if let Some(socket) = endpoint.strip_prefix("unix://") {
        return Ok(Docker::connect_with_unix(socket, 15, API_DEFAULT_VERSION)?);
    }
    #[cfg(windows)]
    if endpoint.starts_with("npipe://") {
        return Ok(Docker::connect_with_named_pipe(
            &named_pipe_path(endpoint)?,
            15,
            API_DEFAULT_VERSION,
        )?);
    }
    bail!("Unsupported Docker endpoint {endpoint}; select a local Docker context for this computer")
}

pub(crate) async fn get(endpoint: &str, path: &str) -> Result<Response<Incoming>> {
    use http_body_util::Empty;
    use hyper::body::Bytes;
    use hyper_util::client::legacy::Client;

    #[cfg(unix)]
    {
        use hyperlocal::{UnixClientExt, UnixConnector, Uri};
        // Raw socket paths are also used by the isolated API fixtures.
        let client: Client<UnixConnector, Empty<Bytes>> = Client::unix();
        Ok(client
            .get(Uri::new(endpoint.trim_start_matches("unix://"), path).into())
            .await?)
    }
    #[cfg(windows)]
    {
        use hyper_named_pipe::{NAMED_PIPE_SCHEME, NamedPipeConnector};
        use hyper_util::rt::TokioExecutor;
        let host = hex::encode(named_pipe_path(endpoint)?);
        let uri = format!("{NAMED_PIPE_SCHEME}://{host}{path}").parse()?;
        let client: Client<NamedPipeConnector, Empty<Bytes>> =
            Client::builder(TokioExecutor::new())
                .pool_max_idle_per_host(0)
                .build(NamedPipeConnector);
        Ok(client.get(uri).await?)
    }
}

#[cfg(any(windows, test))]
fn named_pipe_path(endpoint: &str) -> Result<String> {
    let path = endpoint
        .strip_prefix("npipe://")
        .unwrap_or("")
        .replace('\\', "/");
    let name = path
        .strip_prefix("//./pipe/")
        .filter(|name| !name.is_empty());
    if name.is_none() {
        bail!("Expected a local Docker named pipe (npipe:////./pipe/…)");
    }
    Ok(path.replace('/', "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_desktop_pipes_are_local_and_preserve_the_selected_engine() {
        for name in ["docker_engine", "dockerDesktopLinuxEngine"] {
            assert_eq!(
                named_pipe_path(&format!("npipe:////./pipe/{name}")).unwrap(),
                format!(r"\\.\pipe\{name}")
            );
            assert_eq!(
                named_pipe_path(&format!(r"npipe://\\.\pipe\{name}")).unwrap(),
                format!(r"\\.\pipe\{name}")
            );
        }
        for endpoint in [
            "npipe:////other-host/pipe/docker_engine",
            "npipe:////./pipe/",
            "tcp://localhost:2375",
            "unix:///var/run/docker.sock",
        ] {
            assert!(named_pipe_path(endpoint).is_err(), "{endpoint}");
        }
        assert!(client("tcp://localhost:2375").is_err());
    }
}
