//! Offline metadata analysis of Docker save/export archives. No extraction,
//! Docker connection, content hashing, global state or task runtime is used.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
};

#[derive(Clone, Debug)]
pub struct Limits {
    pub entries: usize,
    pub layers: usize,
    pub metadata_bytes: u64,
    pub path_bytes: usize,
    pub expanded_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 30_000,
            layers: 2_000,
            metadata_bytes: 4 * 1024 * 1024,
            path_bytes: 4096,
            expanded_bytes: 8 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileNode {
    pub name: String,
    pub bytes: u64,
    pub change: String,
    pub children: Vec<FileNode>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Layer {
    pub command: String,
    pub archive_bytes: u64,
    pub unpacked_bytes: u64,
    pub metadata_only: bool,
    pub files: Vec<FileNode>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Finding {
    pub title: String,
    pub detail: String,
    pub hint: String,
    pub heuristic: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LargestFile {
    pub path: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub unpacked_bytes: u64,
    pub wasted_bytes: u64,
    pub efficiency: Option<u64>,
    pub layers: Vec<Layer>,
    pub findings: Vec<Finding>,
    pub largest_files: Vec<LargestFile>,
    pub elapsed_ms: u128,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Manifest {
    config: String,
    layers: Vec<String>,
}
#[derive(Deserialize, Default)]
struct Config {
    #[serde(default)]
    history: Vec<History>,
}
#[derive(Deserialize)]
struct History {
    #[serde(default)]
    created_by: String,
    #[serde(default)]
    empty_layer: bool,
}
#[derive(Clone)]
struct Entry {
    path: String,
    kind: Kind,
    bytes: u64,
}
#[derive(Clone)]
enum Kind {
    Directory,
    File,
    Link(String),
    Symlink,
    Other,
}
#[derive(Clone)]
struct Live {
    directory: bool,
    payload: Option<usize>,
}
struct Payload {
    bytes: u64,
}

fn path(raw: &std::path::Path, limits: &Limits) -> Result<String> {
    let raw = raw.to_str().context("Non UTF-8 archive path")?;
    ensure!(
        raw.len() <= limits.path_bytes,
        "Archive path exceeds byte limit"
    );
    ensure!(
        !raw.starts_with('/'),
        "Absolute archive path is unsupported"
    );
    let parts: Vec<_> = raw
        .split('/')
        .filter(|v| !v.is_empty() && *v != ".")
        .collect();
    ensure!(
        parts.len() <= 64 && !parts.contains(&".."),
        "Unsafe or excessively deep archive path"
    );
    Ok(parts.join("/"))
}
fn checkpoint(cancelled: &impl Fn() -> bool) -> Result<()> {
    ensure!(!cancelled(), "Analysis cancelled");
    Ok(())
}
struct CheckedRead<'a, R, F> {
    inner: R,
    cancelled: &'a F,
    remaining: u64,
}
impl<R: Read, F: Fn() -> bool> Read for CheckedRead<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(std::io::Error::other("Analysis cancelled"));
        }
        // Probe one byte at the limit to distinguish exact EOF from overflow.
        let count = buf
            .len()
            .min(64 * 1024)
            .min(self.remaining.saturating_add(1) as usize);
        let n = self.inner.read(&mut buf[..count])?;
        if n as u64 > self.remaining {
            return Err(std::io::Error::other("Expanded archive exceeds byte limit"));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}
fn json_at<R: Read + Seek, T: serde::de::DeserializeOwned>(
    reader: &mut R,
    index: &BTreeMap<String, (u64, u64)>,
    name: &str,
    limits: &Limits,
) -> Result<T> {
    let &(offset, size) = index
        .get(name)
        .with_context(|| format!("Archive missing {name}"))?;
    ensure!(
        size <= limits.metadata_bytes,
        "Archive JSON exceeds metadata limit"
    );
    reader.seek(SeekFrom::Start(offset))?;
    Ok(serde_json::from_reader(reader.take(size))?)
}

/// Analyze one Docker archive manifest. Seek avoids retaining or extracting
/// layer blobs. `cancelled` is checked throughout indexing, reading and accounting.
/// Bounds fail explicitly; reports are never silently truncated.
pub fn analyze<R: Read + Seek>(
    mut reader: R,
    limits: &Limits,
    cancelled: impl Fn() -> bool,
) -> Result<Report> {
    let started = std::time::Instant::now();
    let mut index = BTreeMap::new();
    let mut index_budget = limits.metadata_bytes;
    {
        let mut archive = tar::Archive::new(&mut reader);
        for item in archive.entries_with_seek()? {
            checkpoint(&cancelled)?;
            let entry = item?;
            let name = path(&entry.path()?, limits)?;
            index_budget = index_budget
                .checked_sub(name.len() as u64)
                .context("Outer paths exceed metadata byte limit")?;
            ensure!(
                index.len() < limits.entries,
                "Outer archive entry limit exceeded"
            );
            ensure!(
                index
                    .insert(name, (entry.raw_file_position(), entry.size()))
                    .is_none(),
                "Duplicate outer archive entry"
            );
        }
    }
    let manifests: Vec<Manifest> = json_at(&mut reader, &index, "manifest.json", limits)?;
    ensure!(
        manifests.len() == 1,
        "Select/export exactly one image for analysis"
    );
    let manifest = &manifests[0];
    ensure!(
        manifest.layers.len() <= limits.layers,
        "Layer limit exceeded"
    );
    let config: Config = json_at(&mut reader, &index, &manifest.config, limits)?;
    ensure!(
        config.history.len() <= limits.layers,
        "History limit exceeded"
    );
    if !config.history.is_empty() {
        ensure!(
            config.history.iter().filter(|h| !h.empty_layer).count() == manifest.layers.len(),
            "History and filesystem layer counts disagree"
        );
    }
    let mut report = Report::default();
    let mut live = BTreeMap::<String, Live>::new();
    let mut payloads = Vec::<Payload>::new();
    let mut retained = 0usize;
    let mut path_budget = limits.metadata_bytes;
    let mut expanded_left = limits.expanded_bytes;
    let mut physical = 0usize;
    let mut node_budget = limits.entries.saturating_mul(2);
    let history = if config.history.is_empty() {
        manifest
            .layers
            .iter()
            .map(|_| History {
                created_by: "Instruction unavailable".into(),
                empty_layer: false,
            })
            .collect()
    } else {
        config.history
    };
    for history in history {
        checkpoint(&cancelled)?;
        let mut layer = Layer {
            command: history.created_by,
            metadata_only: history.empty_layer,
            ..Default::default()
        };
        if !history.empty_layer {
            let name = &manifest.layers[physical];
            physical += 1;
            let &(offset, size) = index
                .get(name)
                .with_context(|| format!("Archive missing layer {name}"))?;
            layer.archive_bytes = size;
            reader.seek(SeekFrom::Start(offset))?;
            let mut magic = [0; 4];
            reader
                .read_exact(&mut magic)
                .context("Layer is too short")?;
            reader.seek(SeekFrom::Start(offset))?;
            let blob = (&mut reader).take(size);
            let decoded: Box<dyn Read + '_> = if magic[..2] == [0x1f, 0x8b] {
                Box::new(flate2::read::MultiGzDecoder::new(blob))
            } else if magic == [0x28, 0xb5, 0x2f, 0xfd] {
                Box::new(zstd::stream::read::Decoder::new(blob)?)
            } else {
                Box::new(blob)
            };
            let mut checked = CheckedRead {
                inner: decoded,
                cancelled: &cancelled,
                remaining: expanded_left,
            };
            let mut entries = Vec::new();
            {
                let mut tar = tar::Archive::new(&mut checked);
                for entry in tar.entries()? {
                    checkpoint(&cancelled)?;
                    let entry = entry?;
                    retained += 1;
                    ensure!(retained <= limits.entries, "Layer entry limit exceeded");
                    let name = path(&entry.path()?, limits)?;
                    path_budget = path_budget
                        .checked_sub(name.len() as u64)
                        .context("Layer paths exceed metadata byte limit")?;
                    if name.is_empty() {
                        continue;
                    }
                    let ty = entry.header().entry_type();
                    ensure!(!ty.is_gnu_sparse(), "Sparse layer files are unsupported");
                    let kind = if ty.is_dir() {
                        Kind::Directory
                    } else if ty.is_file() {
                        Kind::File
                    } else if ty.is_hard_link() {
                        let target = path(
                            &entry.link_name()?.context("Hard link missing target")?,
                            limits,
                        )?;
                        path_budget = path_budget
                            .checked_sub(target.len() as u64)
                            .context("Link targets exceed metadata byte limit")?;
                        Kind::Link(target)
                    } else if ty.is_symlink() {
                        Kind::Symlink
                    } else {
                        Kind::Other
                    };
                    entries.push(Entry {
                        path: name,
                        bytes: if ty.is_file() { entry.size() } else { 0 },
                        kind,
                    });
                }
            }
            expanded_left = checked.remaining;
            let payload_cursor = payloads.len();
            let changes = apply(entries, &mut live, &mut payloads, &cancelled)?;
            ensure!(
                live.len() <= limits.entries * 2,
                "Filesystem node limit exceeded"
            );
            layer.unpacked_bytes = payloads.iter().skip(payload_cursor).map(|p| p.bytes).sum();
            layer.files = tree(&changes, &mut node_budget)?;
            report.unpacked_bytes = report
                .unpacked_bytes
                .checked_add(layer.unpacked_bytes)
                .context("Image byte overflow")?;
        }
        report.layers.push(layer);
    }
    let mut referenced = vec![false; payloads.len()];
    for item in live.values() {
        if let Some(i) = item.payload {
            referenced[i] = true;
        }
    }
    report.wasted_bytes = payloads
        .iter()
        .zip(referenced)
        .filter(|(_, used)| !used)
        .map(|(p, _)| p.bytes)
        .sum();
    report.efficiency = if report.unpacked_bytes == 0 {
        None
    } else {
        Some(
            ((report.unpacked_bytes - report.wasted_bytes) as u128 * 100
                / report.unpacked_bytes as u128) as u64,
        )
    };
    report.largest_files = live
        .iter()
        .filter_map(|(path, item)| {
            item.payload.map(|i| LargestFile {
                path: path.clone(),
                bytes: payloads[i].bytes,
            })
        })
        .collect();
    report
        .largest_files
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.path.cmp(&b.path)));
    report.largest_files.truncate(20);
    if report.wasted_bytes > 0 {
        report.findings.push(Finding { title: "Replaced or deleted payloads".into(), detail: format!("{} bytes from earlier file payloads have no remaining path in the final filesystem.",report.wasted_bytes), hint: "Create and remove temporary files in the same instruction, or copy final artifacts from a build stage.".into(), heuristic: false });
    }
    for file in &report.largest_files {
        report.findings.push(Finding {
            title: "Large final file".into(),
            detail: format!("{} · {:.2} MiB", file.path, file.bytes as f64 / 1_048_576.0),
            ..Default::default()
        });
    }
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}

fn apply(
    entries: Vec<Entry>,
    live: &mut BTreeMap<String, Live>,
    payloads: &mut Vec<Payload>,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<(String, String, u64)>> {
    let mut changes = BTreeMap::new();
    // Whiteouts remove lower-layer state regardless of their tar order. Same
    // layer additions survive opaque markers and ordinary whiteouts.
    for entry in &entries {
        checkpoint(cancelled)?;
        let (parent, base) = entry.path.rsplit_once('/').unwrap_or(("", &entry.path));
        if base == ".wh..wh..opq" {
            remove(parent, false, live, payloads, &mut changes);
        } else if let Some(target) = base.strip_prefix(".wh.") {
            ensure!(
                !target.is_empty() && target != "." && target != "..",
                "Invalid whiteout target"
            );
            let target = if parent.is_empty() {
                target.into()
            } else {
                format!("{parent}/{target}")
            };
            remove(&target, true, live, payloads, &mut changes);
        }
    }
    let mut links: Vec<(String, String)> = Vec::new();
    for entry in entries {
        checkpoint(cancelled)?;
        let base = entry.path.rsplit('/').next().unwrap();
        if base.starts_with(".wh.") {
            continue;
        }
        let prefix = format!("{}/", entry.path);
        links.retain(|(name, _)| name != &entry.path && !name.starts_with(&prefix));
        let existed = live.contains_key(&entry.path) || changes.contains_key(&entry.path);
        let directory = matches!(entry.kind, Kind::Directory);
        if !directory || live.get(&entry.path).is_some_and(|old| !old.directory) {
            remove(&entry.path, true, live, payloads, &mut changes);
        }
        // Materialize implicit parent directories; a child replacing a lower
        // file/symlink removes its old payload instead of retaining impossible state.
        let parts: Vec<_> = entry.path.split('/').collect();
        for n in 1..parts.len() {
            let parent = parts[..n].join("/");
            if live.get(&parent).is_some_and(|v| !v.directory) {
                remove(&parent, true, live, payloads, &mut changes);
                changes.insert(parent.clone(), ("Modified".into(), 0));
            }
            live.entry(parent).or_insert(Live {
                directory: true,
                payload: None,
            });
        }
        let payload = match entry.kind {
            Kind::File => {
                let id = payloads.len();
                payloads.push(Payload { bytes: entry.bytes });
                Some(id)
            }
            Kind::Link(target) => {
                if let Some(id) = live.get(&target).and_then(|v| v.payload) {
                    Some(id)
                } else {
                    links.push((entry.path.clone(), target));
                    None
                }
            }
            _ => None,
        };
        live.insert(entry.path.clone(), Live { directory, payload });
        changes.insert(
            entry.path,
            (
                if existed { "Modified" } else { "Added" }.into(),
                payload.map(|id| payloads[id].bytes).unwrap_or(0),
            ),
        );
    }
    // Resolve forward links and chains without following symlinks or reading
    // host paths. Invalid/missing/cyclic links fail rather than inventing bytes.
    links.retain(|(name, _)| {
        live.get(name)
            .is_some_and(|v| !v.directory && v.payload.is_none())
    });
    let links: BTreeMap<_, _> = links.into_iter().collect();
    for name in links.keys() {
        checkpoint(cancelled)?;
        let mut current = name.clone();
        let mut chain = Vec::new();
        let mut visited = std::collections::BTreeSet::new();
        let payload = loop {
            checkpoint(cancelled)?;
            if let Some(id) = live.get(&current).and_then(|v| v.payload) {
                break id;
            }
            ensure!(visited.insert(current.clone()), "Cyclic hard link target");
            let target = links.get(&current).context("Unresolved hard link target")?;
            chain.push(current);
            current = target.clone();
        };
        for alias in chain {
            live.get_mut(&alias)
                .context("Missing hard link path")?
                .payload = Some(payload);
            changes
                .get_mut(&alias)
                .context("Missing hard link change")?
                .1 = payloads[payload].bytes;
        }
    }
    Ok(changes.into_iter().map(|(p, (c, b))| (p, c, b)).collect())
}
fn remove(
    target: &str,
    itself: bool,
    live: &mut BTreeMap<String, Live>,
    payloads: &[Payload],
    changes: &mut BTreeMap<String, (String, u64)>,
) {
    let prefix = if target.is_empty() {
        String::new()
    } else {
        format!("{target}/")
    };
    let mut keys: Vec<_> = live
        .range(prefix.clone()..)
        .take_while(|(p, _)| p.starts_with(&prefix))
        .map(|(p, _)| p.clone())
        .collect();
    if itself && live.contains_key(target) {
        keys.push(target.to_owned());
    }
    for key in keys {
        let old = live.remove(&key).unwrap();
        changes.insert(
            key,
            (
                "Removed".into(),
                old.payload.map(|i| payloads[i].bytes).unwrap_or(0),
            ),
        );
    }
}
fn tree(changes: &[(String, String, u64)], budget: &mut usize) -> Result<Vec<FileNode>> {
    #[derive(Default)]
    struct Branch {
        row: Option<(String, u64)>,
        children: BTreeMap<String, Branch>,
    }
    fn nodes(branch: Branch) -> Vec<FileNode> {
        branch
            .children
            .into_iter()
            .map(|(name, b)| {
                let (change, bytes) = b.row.unwrap_or(("Changed descendants".into(), 0));
                FileNode {
                    name,
                    bytes,
                    change,
                    children: nodes(Branch {
                        row: None,
                        children: b.children,
                    }),
                }
            })
            .collect()
    }
    let mut root = Branch::default();

    for (path, change, bytes) in changes {
        let mut b = &mut root;
        for part in path.split('/') {
            if !b.children.contains_key(part) {
                *budget = budget
                    .checked_sub(1)
                    .context("File tree node limit exceeded")?;
            }
            b = b.children.entry(part.into()).or_default();
        }
        b.row = Some((change.clone(), *bytes));
    }
    Ok(nodes(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn tar(entries: &[(&str, u8, &[u8], &str)]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, kind, bytes, link) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_mode(0o644);
            h.set_size(bytes.len() as u64);
            h.set_entry_type(tar::EntryType::new(*kind));
            if !link.is_empty() {
                h.set_link_name(link).unwrap();
            }
            h.set_cksum();
            tar.append_data(&mut h, *path, *bytes).unwrap();
        }
        tar.into_inner().unwrap()
    }
    fn image(layers: Vec<Vec<u8>>, history: serde_json::Value) -> Vec<u8> {
        let config = serde_json::to_vec(&serde_json::json!({"history":history})).unwrap();
        let names: Vec<_> = (0..layers.len())
            .map(|i| format!("{i}/layer.tar"))
            .collect();
        let manifest = serde_json::to_vec(
            &serde_json::json!([{"Config":"config.json","RepoTags":null,"Layers":names}]),
        )
        .unwrap();
        let mut entries = vec![
            ("manifest.json", b'0', manifest.as_slice(), ""),
            ("config.json", b'0', config.as_slice(), ""),
        ];
        for (name, layer) in names.iter().zip(&layers) {
            entries.push((name, b'0', layer, ""));
        }
        tar(&entries)
    }
    fn run(layers: Vec<Vec<u8>>) -> Report {
        analyze(
            Cursor::new(image(layers, serde_json::json!([]))),
            &Limits::default(),
            || false,
        )
        .unwrap()
    }
    #[test]
    fn replaced_and_deleted_payloads() {
        let r = run(vec![
            tar(&[("app/a", b'0', b"12345", ""), ("app/b", b'0', b"123", "")]),
            tar(&[("app/.wh.b", b'0', b"", ""), ("app/a", b'0', b"12", "")]),
        ]);
        assert_eq!(
            (r.unpacked_bytes, r.wasted_bytes, r.efficiency),
            (10, 8, Some(20))
        );
        assert_eq!(r.largest_files[0].path, "app/a");
        assert_eq!(r.largest_files[0].bytes, 2);
    }
    #[test]
    fn opaque_preserves_same_layer_additions_in_any_order() {
        for reverse in [false, true] {
            let mut entries = vec![
                ("d/new", b'0', b"123".as_slice(), ""),
                ("d/.wh..wh..opq", b'0', b"".as_slice(), ""),
            ];
            if reverse {
                entries.reverse();
            }
            let r = run(vec![
                tar(&[("d/old", b'0', b"12345", ""), ("keep", b'0', b"1", "")]),
                tar(&entries),
            ]);
            assert_eq!(r.wasted_bytes, 5);
            assert_eq!(r.largest_files[0].path, "d/new");
        }
    }
    #[test]
    fn directory_removal_and_file_directory_replacement() {
        let r = run(vec![
            tar(&[
                ("d/a", b'0', b"12345", ""),
                ("d/sub/b", b'0', b"12", ""),
                ("file", b'0', b"123", ""),
            ]),
            tar(&[("d", b'0', b"1", ""), ("file/child", b'0', b"1", "")]),
        ]);
        assert_eq!(r.wasted_bytes, 10);
        assert_eq!(r.unpacked_bytes, 12);
        let r = run(vec![
            tar(&[("d/a", b'0', b"12345", "")]),
            tar(&[(".wh.d", b'0', b"", "")]),
        ]);
        assert_eq!(r.wasted_bytes, 5);
    }
    #[test]
    fn directory_metadata_does_not_remove_children() {
        let r = run(vec![
            tar(&[("d/a", b'0', b"123", "")]),
            tar(&[("d", b'5', b"", "")]),
        ]);
        assert_eq!(r.wasted_bytes, 0);
    }
    #[test]
    fn hardlinks_keep_payload_until_last_alias_removed() {
        let first = tar(&[("a", b'0', b"12345", ""), ("alias", b'1', b"", "a")]);
        let r = run(vec![first.clone(), tar(&[(".wh.a", b'0', b"", "")])]);
        assert_eq!(r.wasted_bytes, 0);
        assert_eq!(r.unpacked_bytes, 5);
        assert_eq!(r.largest_files[0].path, "alias");
        let r = run(vec![
            first,
            tar(&[(".wh.a", b'0', b"", ""), (".wh.alias", b'0', b"", "")]),
        ]);
        assert_eq!(r.wasted_bytes, 5);
    }
    #[test]
    fn forward_hardlink_and_symlink_are_not_host_reads() {
        let r = run(vec![tar(&[
            ("alias", b'1', b"", "a"),
            ("a", b'0', b"123", ""),
            ("outside", b'2', b"", "/etc/passwd"),
        ])]);
        assert_eq!(r.unpacked_bytes, 3);
        assert_eq!(r.wasted_bytes, 0);
        let bad = image(
            vec![tar(&[("a", b'1', b"", "b"), ("b", b'1', b"", "a")])],
            serde_json::json!([]),
        );
        assert!(analyze(Cursor::new(bad), &Limits::default(), || false).is_err());
    }
    #[test]
    fn metadata_history_and_zero_efficiency_unavailable() {
        let bytes = image(
            vec![tar(&[])],
            serde_json::json!([{"created_by":"LABEL x=y","empty_layer":true},{"created_by":"COPY empty"},{"created_by":"CMD []","empty_layer":true}]),
        );
        let r = analyze(Cursor::new(bytes), &Limits::default(), || false).unwrap();
        assert_eq!(r.layers.len(), 3);
        assert!(r.layers[0].metadata_only);
        assert_eq!(r.efficiency, None);
    }
    #[test]
    fn gzip_and_zstd_layer_streams() {
        use std::io::Write;
        let bytes = tar(&[("file", b'0', b"123", "")]);
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&bytes).unwrap();
        for layer in [
            gzip.finish().unwrap(),
            zstd::stream::encode_all(bytes.as_slice(), 1).unwrap(),
        ] {
            assert_eq!(run(vec![layer]).unpacked_bytes, 3);
        }
    }
    #[test]
    fn limits_cancellation_and_mismatched_history_fail() {
        let bytes = image(
            vec![tar(&[("file", b'0', b"123", "")])],
            serde_json::json!([]),
        );
        assert!(analyze(Cursor::new(&bytes), &Limits::default(), || true).is_err());
        assert!(
            analyze(
                Cursor::new(&bytes),
                &Limits {
                    expanded_bytes: 100,
                    ..Default::default()
                },
                || false
            )
            .is_err()
        );
        assert!(
            analyze(
                Cursor::new(&bytes),
                &Limits {
                    entries: 1,
                    ..Default::default()
                },
                || false
            )
            .is_err()
        );
        let bad = image(vec![tar(&[])], serde_json::json!([{"empty_layer":true}]));
        assert!(analyze(Cursor::new(bad), &Limits::default(), || false).is_err());
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use std::{cell::Cell, io::Cursor};
    #[test]
    fn directory_prefixes_do_not_stop_removal_at_a_similar_sibling() {
        let mut live = BTreeMap::from([
            (
                "d/a".into(),
                Live {
                    directory: false,
                    payload: Some(0),
                },
            ),
            (
                "d-keep".into(),
                Live {
                    directory: false,
                    payload: Some(1),
                },
            ),
        ]);
        let mut changes = BTreeMap::new();
        remove(
            "d",
            true,
            &mut live,
            &[Payload { bytes: 5 }, Payload { bytes: 3 }],
            &mut changes,
        );
        assert_eq!(live.len(), 1);
        assert!(live.contains_key("d-keep"));
        assert_eq!(changes["d/a"].1, 5);
    }
    #[test]
    fn replacing_target_keeps_an_earlier_hardlink_payload() {
        let mut live = BTreeMap::new();
        let mut payloads = Vec::new();
        let entries = vec![
            Entry {
                path: "a".into(),
                kind: Kind::File,
                bytes: 5,
            },
            Entry {
                path: "alias".into(),
                kind: Kind::Link("a".into()),
                bytes: 0,
            },
            Entry {
                path: "a".into(),
                kind: Kind::File,
                bytes: 2,
            },
        ];
        apply(entries, &mut live, &mut payloads, &|| false).unwrap();
        assert_eq!(live["alias"].payload, Some(0));
        assert_eq!(live["a"].payload, Some(1));
    }
    #[test]
    fn overwritten_forward_link_does_not_panic_or_restore_old_link() {
        let mut live = BTreeMap::new();
        let mut payloads = Vec::new();
        let entries = vec![
            Entry {
                path: "alias".into(),
                kind: Kind::Link("missing".into()),
                bytes: 0,
            },
            Entry {
                path: "alias".into(),
                kind: Kind::File,
                bytes: 2,
            },
        ];
        apply(entries, &mut live, &mut payloads, &|| false).unwrap();
        assert_eq!(live["alias"].payload, Some(0));
    }
    #[test]
    fn cancelled_reader_stops_between_payload_chunks() {
        let cancelled = Cell::new(false);
        let mut r = CheckedRead {
            inner: Cursor::new(vec![1; 128 * 1024]),
            cancelled: &|| cancelled.get(),
            remaining: 128 * 1024,
        };
        let mut buf = vec![0; 128 * 1024];
        assert_eq!(r.read(&mut buf).unwrap(), 64 * 1024);
        cancelled.set(true);
        assert!(
            r.read(&mut buf)
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }
    #[test]
    fn final_tree_budget_is_shared_across_layers() {
        let rows = vec![("a/b/c".into(), "Added".into(), 1)];
        let mut budget = 4;
        assert!(tree(&rows, &mut budget).is_ok());
        assert_eq!(budget, 1);
        assert!(tree(&rows, &mut budget).is_err());
    }
}
