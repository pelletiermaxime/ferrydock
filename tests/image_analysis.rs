//! Real Docker adapter and worker proof, independently of GPUI rendering.
use anyhow::{Context, Result};
use ferrydock::{
    docker::{self, Backend, Command, Event},
    image_analysis,
};
use std::{env, time::Duration};
use tokio::time;
#[tokio::test]
#[ignore = "Owned image supplied by scripts/verify-image-analysis.sh"]
async fn exported_image_analysis() -> Result<()> {
    let id = env::var("FERRYDOCK_ANALYSIS_IMAGE")?;
    let connection = docker::resolve_connection().await?;
    assert_eq!(
        connection.endpoint,
        env::var("FERRYDOCK_ANALYSIS_ENDPOINT")?
    );
    let docker = docker::connect(&connection).await?;
    let inspect = docker.inspect_image(&id).await?;
    assert_eq!(
        inspect
            .config
            .and_then(|c| c.labels)
            .and_then(|l| l.get("dev.ferrydock.fixture.project").cloned()),
        Some(env::var("FERRYDOCK_ANALYSIS_PROJECT")?)
    );
    let (backend, mut events) = Backend::start();
    time::timeout(Duration::from_secs(25), async {
        loop {
            if let Event::Connected(_) = events.recv().await.context("Worker ended")? {
                return Ok::<_, anyhow::Error>(());
            }
        }
    })
    .await??;
    backend.send(Command::AnalyzeImage {
        id: id.clone(),
        generation: 77,
    });
    let report = time::timeout(Duration::from_secs(125), async {
        loop {
            if let Event::ImageAnalysis {
                id: current,
                generation: 77,
                result,
            } = events.recv().await.context("Worker ended")?
            {
                assert_eq!(current, id);
                return result.map_err(anyhow::Error::msg);
            }
        }
    })
    .await??;
    assert_eq!(report.wasted_bytes, 1_048_576);
    assert!(report.layers.iter().any(|l| l.metadata_only));
    fn contains(nodes: &[image_analysis::FileNode], change: &str) -> bool {
        nodes
            .iter()
            .any(|n| n.name == "discard.bin" && n.change == change || contains(&n.children, change))
    }
    assert!(report.layers.iter().any(|l| contains(&l.files, "Added")));
    assert!(report.layers.iter().any(|l| contains(&l.files, "Removed")));
    assert!(report.largest_files.iter().all(|f| f.path != "discard.bin"));
    assert!(
        image_analysis::analyze(
            &docker,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        )
        .await
        .is_err()
    );
    std::fs::write(
        env::var("FERRYDOCK_ANALYSIS_REPORT")?,
        serde_json::to_vec_pretty(&*report)?,
    )?;
    backend.send(Command::CloseImageAnalysis);
    drop(backend);
    println!(
        "Production worker export analysis passed: {} wasted bytes, {:?}% efficiency",
        report.wasted_bytes, report.efficiency
    );
    Ok(())
}
