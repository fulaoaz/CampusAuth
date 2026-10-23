#!/usr/bin/env bash
set -euo pipefail

desktop_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
project_dir="$(CDPATH= cd -- "$desktop_dir/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$desktop_dir/target}"
if [[ "$target_dir" != /* ]]; then target_dir="$desktop_dir/$target_dir"; fi
profile_dir="$target_dir"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then profile_dir="$profile_dir/$CARGO_BUILD_TARGET"; fi
bundle_dir="${1:-$profile_dir/release/bundle/appimage}"
app_dir="$bundle_dir/CampusAuth.AppDir"
original_helper="$desktop_dir/resources/campus-auth-power-helper"
bundled_helper="$app_dir/usr/lib/CampusAuth/resources/campus-auth-power-helper"
version="$(tr -d '\r\n' < "$project_dir/VERSION")"
tool_dir="${TAURI_BUNDLER_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/tauri}"
output_plugin="$tool_dir/linuxdeploy-plugin-appimage.AppImage"

for command in file sha256sum cmp install mktemp; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf '构建环境缺少 %s。\n' "$command" >&2
    exit 2
  fi
done
if [[ ! -d "$app_dir" || ! -f "$bundled_helper" || ! -f "$original_helper" ]]; then
  printf '%s\n' '请先构建 Linux AppImage，保留 AppDir 与静态电源辅助程序。' >&2
  exit 2
fi
if ! file "$original_helper" | grep -Eq 'static(-pie)? linked|statically linked'; then
  printf '%s\n' '电源辅助程序必须是 musl 静态 ELF，停止封装。' >&2
  exit 2
fi
if [[ ! -x "$output_plugin" ]]; then
  printf '找不到 Tauri 官方缓存的 AppImage 输出插件：%s\n' "$output_plugin" >&2
  printf '%s\n' '请先运行 Tauri AppImage 构建，或用 TAURI_BUNDLER_TOOLS_DIR 指定其缓存目录。' >&2
  exit 2
fi

# linuxdeploy's GTK plugin rewrites every ELF rpath, which corrupts a static
# PIE helper. Restore the already-built static resource after that pass.
original_hash="$(sha256sum "$original_helper" | cut -d ' ' -f 1)"
install -m 755 "$original_helper" "$bundled_helper"
if ! cmp -s "$original_helper" "$bundled_helper"; then
  printf '%s\n' 'AppDir 电源辅助程序校验失败，停止封装。' >&2
  exit 2
fi
"$bundled_helper" --version
preview_dir="$(mktemp -d -t campus-auth-helper-preview.XXXXXXXX)"
trap 'rmdir "$preview_dir" 2>/dev/null || true' EXIT
"$bundled_helper" --data-dir "$preview_dir" power preview >/dev/null

case "$(uname -m)" in
  x86_64) architecture=x86_64; package_arch=amd64 ;;
  aarch64) architecture=aarch64; package_arch=aarch64 ;;
  *) printf '%s\n' '当前 AppImage 架构未受支持。' >&2; exit 2 ;;
esac
output="$bundle_dir/CampusAuth_${version}_${package_arch}.AppImage"
# A cached official type2 runtime avoids an optional network fetch by
# appimagetool. LDAI_RUNTIME_FILE can also point at a separately verified copy.
if [[ -z "${LDAI_RUNTIME_FILE:-}" && -f "$tool_dir/runtime-$architecture" ]]; then
  export LDAI_RUNTIME_FILE="$tool_dir/runtime-$architecture"
fi
# Only invoke the official output stage; rerunning the GTK dependency pass
# here would rewrite and break the restored helper again.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$architecture" LDAI_OUTPUT="$output" \
  "$output_plugin" --appdir "$app_dir"
if ! cmp -s "$original_helper" "$bundled_helper"; then
  printf '%s\n' '输出插件改变了静态电源辅助程序，停止交付。' >&2
  exit 2
fi
printf 'AppImage 静态辅助程序已校验：%s\n' "$original_hash"
printf 'AppImage 已生成：%s\n' "$output"
