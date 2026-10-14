//! Resolve exactly which services have generated image names before teardown.
//! Never delegate image scope to version-dependent `compose down --rmi`.
use super::{Action, Config};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
struct Model {
    services: BTreeMap<String, Service>,
}
#[derive(Deserialize)]
struct Service {
    image: Option<String>,
    build: Option<serde_json::Value>,
}

fn image_names(project: &str, bytes: &[u8]) -> Result<Vec<String>> {
    let model: Model =
        serde_json::from_slice(bytes).context("Cannot read Compose image cleanup plan")?;
    if model.services.len() > 2000 {
        bail!("Compose image cleanup plan exceeds 2,000 services");
    }
    Ok(model
        .services
        .into_iter()
        .filter_map(|(name, service)| {
            (service.build.is_some() && service.image.as_deref().is_none_or(str::is_empty))
                .then(|| format!("{project}-{name}"))
        })
        .collect())
}

async fn bounded_output(reader: impl AsyncRead + Unpin, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > limit {
        bail!("Compose image cleanup plan output is too large");
    }
    Ok(bytes)
}

async fn capture(
    config: &Config,
    endpoint: &str,
    args: Vec<String>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>> {
    let mut child = super::command(config, endpoint, args)
        .spawn()
        .context("Cannot inspect Compose image cleanup plan")?;
    let stdout = child.stdout.take().context("Missing Compose plan stdout")?;
    let stderr = child.stderr.take().context("Missing Compose plan stderr")?;
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(anyhow::anyhow!("Compose cleanup cancelled before teardown")),
        result = tokio::time::timeout(Duration::from_secs(15), async {
            let (out, err) = tokio::try_join!(bounded_output(stdout, 4 * 1024 * 1024), bounded_output(stderr, 64 * 1024))?;
            let status = child.wait().await?;
            if !status.success() { bail!("Cannot inspect Compose image cleanup plan: {}", String::from_utf8_lossy(&err)); }
            Ok(out)
        }) => result.context("Compose image cleanup plan timed out").and_then(|result| result),
    };
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

pub(super) async fn generated_images(
    config: &Config,
    endpoint: &str,
    cancel: &CancellationToken,
) -> Result<Vec<String>> {
    let action = Action::Cleanup {
        volumes: false,
        images: false,
    };
    let mut args = config.args(endpoint, action);
    args.truncate(args.len() - action.args().len());
    args.extend(["config".into(), "--format".into(), "json".into()]);
    let model = capture(config, endpoint, args, cancel).await?;
    let names = image_names(&config.project, &model)?;
    if names.is_empty() {
        return Ok(names);
    }
    let args = vec![
        "--host".into(),
        endpoint.into(),
        "image".into(),
        "ls".into(),
        "--filter".into(),
        format!("reference={}-*", config.project),
        "--format".into(),
        "{{json .}}".into(),
    ];
    let listing = capture(config, endpoint, args, cancel).await?;
    let mut existing = std::collections::BTreeSet::new();
    for line in std::str::from_utf8(&listing)?.lines() {
        #[derive(Deserialize)]
        struct Tag {
            #[serde(rename = "Repository")]
            repository: String,
            #[serde(rename = "Tag")]
            tag: String,
        }
        let image: Tag =
            serde_json::from_str(line).context("Cannot read generated image inventory")?;
        if image.tag == "latest" {
            existing.insert(image.repository);
        }
    }
    Ok(names
        .into_iter()
        .filter(|name| existing.contains(name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_generated_build_names_are_removed() {
        let model = br#"{"services":{"local":{"build":{"context":"."}},"tagged":{"build":{"context":"."},"image":"custom:dev"},"pulled":{"image":"alpine:3.23"}}}"#;
        assert_eq!(image_names("owned", model).unwrap(), vec!["owned-local"]);
        assert!(image_names("owned", br#"{"services": []}"#).is_err());
    }
}
