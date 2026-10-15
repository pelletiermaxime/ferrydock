//! An owned HTTP fixture over the host's local Docker transport, never a daemon.
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct LocalApi {
    pub endpoint: String,
    task: tokio::task::JoinHandle<()>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
    _directory: tempfile::TempDir,
}

impl LocalApi {
    pub fn new(expected_request: String, status: &str, response: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let (endpoint, listener) = {
            let path = directory.path().join("api.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            (format!("unix://{}", path.display()), listener)
        };
        #[cfg(windows)]
        let (endpoint, listener) = {
            let name = format!(
                "ferrydock-api-{}-{}",
                std::process::id(),
                directory.path().file_name().unwrap().to_string_lossy()
            );
            let listener = tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(format!(r"\\.\pipe\{name}"))
                .unwrap();
            (format!("npipe:////./pipe/{name}"), listener)
        };
        let status = status.to_owned();
        let response = response.to_owned();
        let (release, finished) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                #[cfg(unix)]
                let (stream, _) = listener.accept().await.unwrap();
                #[cfg(windows)]
                let stream = {
                    listener.connect().await.unwrap();
                    listener
                };
                // Windows must keep the server handle open until the client has
                // consumed the response; closing a pipe can discard unread data.
                let _stream = serve(stream, &expected_request, &status, &response).await;
                let _ = finished.await;
            })
            .await
            .expect("local API fixture timed out");
        });
        Self {
            endpoint,
            task,
            release: Some(release),
            _directory: directory,
        }
    }

    pub async fn finish(&mut self) {
        let _ = self.release.take().unwrap().send(());
        (&mut self.task).await.unwrap();
    }
}

impl Drop for LocalApi {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    expected_request: &str,
    status: &str,
    response: &str,
) -> S {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0 && request.len() + count <= 16 * 1024);
        request.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8(request).unwrap();
    assert_eq!(request.lines().next().unwrap(), expected_request);
    stream
        .write_all(
            format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes(),
        )
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
    stream
}
