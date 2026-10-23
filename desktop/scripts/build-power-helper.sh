#!/usr/bin/env sh
set -eu
desktop_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
root_dir=$(CDPATH='' cd -- "$desktop_dir/.." && pwd)
target=${CAMPUS_HELPER_TARGET:-x86_64-unknown-linux-musl}
build_dir=${CAMPUS_HELPER_BUILD_DIR:-"$root_dir/.build-work/power-helper"}

case "$target" in
  x86_64-unknown-linux-musl|aarch64-unknown-linux-musl) ;;
  *) printf '%s\n' '电源辅助程序需要 Linux musl 静态目标。' >&2; exit 2 ;;
esac

# Build-time requirements belong to the developer/CI environment. The shipped
# helper has no GTK, WebKit, Python, Node.js, or application-server dependency.
CARGO_TARGET_DIR="$build_dir" cargo build --manifest-path "$root_dir/Cargo.toml" \
  --locked --release --no-default-features --target "$target"
mkdir -p "$desktop_dir/resources"
cp "$build_dir/$target/release/campus-auth" "$desktop_dir/resources/campus-auth-power-helper"
chmod 755 "$desktop_dir/resources/campus-auth-power-helper"
printf '%s\n' 'Linux 静态电源辅助程序已准备。'
