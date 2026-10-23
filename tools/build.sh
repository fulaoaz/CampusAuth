#!/usr/bin/env bash
set -euo pipefail
project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"
mode="${1:-server}"
mkdir -p dist
case "$mode" in
  desktop)
    cd desktop
    npm run build:linux
    ;;
  server)
    cargo build --release --locked --target x86_64-unknown-linux-musl
    install -m 755 target/x86_64-unknown-linux-musl/release/campus-auth dist/campus-auth-server-linux-amd64
    ;;
  server-arm64)
    cargo build --release --locked --target aarch64-unknown-linux-musl
    install -m 755 target/aarch64-unknown-linux-musl/release/campus-auth dist/campus-auth-server-linux-arm64
    ;;
  *) printf '用法：%s [desktop|server|server-arm64]\n' "$0" >&2; exit 2 ;;
esac
