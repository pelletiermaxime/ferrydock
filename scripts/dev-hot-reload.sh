#!/usr/bin/env bash
# GPUI's Subsecond integration and the development server use the same protocol.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ferrydock_dx="${FERRYDOCK_DX:-dx}"
if ! command -v "$ferrydock_dx" >/dev/null 2>&1; then
    echo 'Install Dioxus CLI 0.7.10: cargo install dioxus-cli --version 0.7.10 --locked' >&2
    exit 1
fi
if [[ "$("$ferrydock_dx" --version)" != 'dioxus 0.7.10 '* ]]; then
    echo 'FerryDock hot reload requires Dioxus CLI 0.7.10 to match GPUI Fast.' >&2
    exit 1
fi

export RUST_LOG="${RUST_LOG:-warn,gpui=info}"
exec "$ferrydock_dx" serve --platform desktop --renderer native \
    --package ferrydock-dev --bin ferrydock-dev --features hot-reload \
    --hot-patch --cargo-args=--locked "$@"
