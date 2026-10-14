# image-analysis-core

A bounded offline reader for a single-image Docker save/export archive. It reads
manifest/config history, streams plain/Gzip/Zstd layer metadata, applies whiteouts
and path replacements, and returns layer changes, regular file payload bytes,
estimated waste and the twenty largest final file paths. It never extracts
files, hashes content, accesses Docker/network/global configuration or schedules
tasks. The implementation is original; no diving source is included.

Call `analyze(reader, &limits, cancelled)` with a `Read + Seek` archive reader and
a cancellation callback. Standard limits are explicit and configurable through
`Limits`. Failures return an error rather than a partial/truncated report. See
FerryDock's development guide for the app's scheduling and limits.

Waste is the sum of regular file payloads with no path in the final filesystem.
Hard links share a payload, and symlinks are not followed. Replacing an identical
file still counts its previous payload: there is no duplicate-content hashing.
Directory metadata retains descendants; non-directory replacement removes them.
Opaque and named whiteouts remove lower-layer paths before same-layer additions,
regardless of tar order. Efficiency is unavailable when total payload bytes are
zero. File tree parent rows with `Changed descendants` are structural containers
and have no payload of their own. Largest-file paths can include hard-link aliases;
image totals count each payload once.

Supports an uncompressed Docker outer tar with exactly one `manifest.json`
record. Configuration history maps metadata-only entries without consuming
filesystem layers. Missing history labels instructions unavailable. OCI-only
layouts without Docker's compatibility manifest, compressed outer archives,
sparse files, non-UTF-8/unsafe/deep paths and invalid hard links fail explicitly.
Tar overhead, file attributes, device contents, compressed byte savings and
Docker shared-layer disk recovery are outside the waste estimate.

Run `cargo test -p image-analysis-core` for semantic and boundary tests.
