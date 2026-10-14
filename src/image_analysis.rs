//! Docker export and cancellable blocking analysis. The core owns no scheduling.
use crate::docker::Event;
use anyhow::{Context, Result, bail};
use bollard::Docker;
use futures_util::StreamExt;
pub use image_analysis_core::{FileNode, Finding, Layer, Report};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    sync::{Semaphore, mpsc, watch},
};
use tokio_util::sync::CancellationToken;

const MAX_EXPORT: u64 = 2 * 1024 * 1024 * 1024;
const MAX_REPORT: usize = 16 * 1024 * 1024;
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub async fn analyze(docker: &Docker, id: &str) -> Result<Report> {
    analyze_with_gate(docker, id, Arc::new(Semaphore::new(1))).await
}

async fn analyze_with_gate(docker: &Docker, id: &str, gate: Arc<Semaphore>) -> Result<Report> {
    let cancel = CancellationToken::new();
    let _guard = CancelOnDrop(cancel.clone());
    tokio::time::timeout(Duration::from_secs(120), async {
        let temp = tempfile::Builder::new()
            .prefix("ferrydock-analysis-")
            .tempdir()?;
        let archive = temp.path().join("image.tar");
        let mut file = tokio::fs::File::create(&archive).await?;
        let mut stream = docker.export_image(id);
        let mut size = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Docker image export failed")?;
            size = size.saturating_add(chunk.len() as u64);
            if size > MAX_EXPORT {
                bail!("Image export exceeds the limit (2 GiB)");
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);
        let permit = gate.acquire_owned().await.context("Analysis gate closed")?;
        // Own the temp directory until the blocking reader actually exits, even
        // if its async waiter is cancelled. No detached parser can lose its file.
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _temp = temp;
            let report = image_analysis_core::analyze(
                std::fs::File::open(archive)?,
                &Default::default(),
                || cancel.is_cancelled(),
            )?;
            if serde_json::to_vec(&report)?.len() > MAX_REPORT {
                bail!("Analysis exceeds retained report limit (16 MiB)");
            }
            Ok(report)
        })
        .await
        .context("Analysis task failed")?
    })
    .await
    .context("Image analysis timed out after two minutes")?
}

pub async fn follow(
    docker: Docker,
    mut requests: watch::Receiver<Option<(String, u64)>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let gate = Arc::new(Semaphore::new(1));
    loop {
        let request = requests.borrow_and_update().clone();
        if let Some((id, generation)) = request {
            tokio::select! {
                _ = cancel.cancelled() => return,
                changed = requests.changed() => { if changed.is_err() { return; } continue; },
                result = analyze_with_gate(&docker,&id,gate.clone()) => {
                    let result=result.map(Arc::new).map_err(|e|format!("{e:#}"));
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        changed = requests.changed() => { if changed.is_err() { return; } continue; },
                        _ = events.send(Event::ImageAnalysis { id,generation,result }) => {},
                    }
                }
            }
        }
        tokio::select! { _ = cancel.cancelled()=>return, changed=requests.changed()=>if changed.is_err(){return;} }
    }
}
