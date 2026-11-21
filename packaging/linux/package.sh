#!/usr/bin/env bash
# Builds the Linux download: setu-<version>-linux-x64.tar.gz in target/package.
# Run from the repo root after `npm ci` in app/.
set -euo pipefail
version=${1:?usage: packaging/linux/package.sh <version>}
name="setu-$version-linux-x64"
out="target/package/$name"
cargo build --release --locked -p setu-cli
(cd app && npx tauri build --no-bundle)
rm -rf "$out" && mkdir -p "$out"
cp target/release/setu target/release/setu-watch target/release/setu-app "$out/"
cp packaging/linux/install.sh packaging/linux/setu.desktop packaging/linux/setu-watch.service "$out/"
cp app/src-tauri/icons/128x128.png "$out/setu.png"
cp LICENSE "$out/"
tar -C target/package -czf "target/package/$name.tar.gz" "$name"
(cd target/package && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "target/package/$name.tar.gz"
