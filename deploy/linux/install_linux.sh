#!/usr/bin/env bash
set -euo pipefail

# 在线与离线部署都只运行已编译的 Rust 程序，不使用 Python 或 Node.js。
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
ACTION=install
PREVIEW=0
BINARY="${CAMPUS_AUTH_BINARY:-}"
BASE_URL="${CAMPUS_AUTH_RELEASE_BASE_URL:-https://github.com/fulaoaz/CampusAuth/releases/latest/download}"
COMPONENTS=
LAN_PROMPT=0
ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY="${2:?--binary 需要程序路径}"; shift 2 ;;
    --action) ACTION="${2:?--action 需要 install/update/rollback/uninstall/tui}"; shift 2 ;;
    --components|--mode) COMPONENTS="${2:?请选择 all/webui/tui/core}"; shift 2 ;;
    --tui) ACTION=tui; shift ;;
    --dry-run) PREVIEW=1; shift ;;
    --allow-insecure-lan) LAN_PROMPT=1; ARGS+=("--allow-insecure-lan"); shift ;;
    --help|-h)
      cat <<'HELP'
CampusAuth Linux 服务器安装器
用法：install_linux.sh [--binary 程序路径] [--action 操作] [--components 模式] [服务参数]
模式：all   = WebUI + TUI + 后台认证（默认）
      webui = WebUI + 后台认证
      tui   = TUI + 后台认证，不监听 HTTP
      core  = 仅后台认证，不监听 HTTP
操作：install、update、rollback、uninstall、tui
离线：把安装器和对应 CPU 的 campus-auth-server-linux-amd64/arm64 放在同一目录，或使用 --binary。
在线：自动下载公开发行程序，并校验 SHA256SUMS。
--dry-run 只预览；卸载默认保留数据，--purge 明确清除已知 CampusAuth 数据。
默认 WebUI 仅监听 127.0.0.1:8765；远程访问请使用 HTTPS 反向代理或 SSH 转发。
可信局域网可明确使用 --host 0.0.0.0 --allow-insecure-lan；HTTP 会明文传输管理密码。
首次管理密码请在本机设置，安装完成后会显示命令。
HELP
      exit 0 ;;
    *) ARGS+=("$1"); shift ;;
  esac
done
[[ "$(uname -s)" == Linux ]] || { echo '此安装器只支持 Linux。' >&2; exit 2; }
case "$ACTION" in install|update|rollback|uninstall|tui) ;; *) echo '不支持的操作。' >&2; exit 2 ;; esac
if [[ -z "$COMPONENTS" && "$ACTION" == install && -t 0 ]]; then
  printf '\n请选择安装的服务器组件：\n  1. 完整安装：WebUI + TUI + 后台认证\n  2. WebUI + 后台认证\n  3. TUI + 后台认证（无 HTTP）\n  4. 仅后台认证（无 HTTP）\n'
  read -r -p '输入 1–4，回车选择 1：' choice
  case "${choice:-1}" in 1) COMPONENTS=all;; 2) COMPONENTS=webui;; 3) COMPONENTS=tui;; 4) COMPONENTS=core;; *) echo '选择无效。' >&2; exit 2;; esac
  if [[ "$COMPONENTS" == all || "$COMPONENTS" == webui ]]; then
    read -r -p '允许可信局域网访问？HTTP 会明文传输管理密码，推荐 HTTPS 代理。[y/N]：' lan
    case "$lan" in y|Y) ARGS+=(--host 0.0.0.0 --allow-insecure-lan); LAN_PROMPT=1;; esac
  fi
fi
COMPONENTS="${COMPONENTS:-all}"
case "$COMPONENTS" in all|webui|tui|core) ;; *) echo '组件模式必须是 all、webui、tui 或 core。' >&2; exit 2 ;; esac
if [[ "$LAN_PROMPT" == 1 ]]; then
  echo '已选择局域网 HTTP 访问；服务将拒绝公网客户端。请仅在可信网络开放，互联网访问请使用 HTTPS 代理。'
fi
case "$(uname -m)" in x86_64|amd64) ARCH=amd64;; aarch64|arm64) ARCH=arm64;; *) echo '当前 CPU 尚无预编译发行包，请提供兼容程序。' >&2; exit 2;; esac
NAME="campus-auth-server-linux-$ARCH"
if [[ -z "$BINARY" ]]; then
  for candidate in "$SCRIPT_DIR/$NAME" "$SCRIPT_DIR/campus-auth" "$ROOT_DIR/$NAME" "$ROOT_DIR/campus-auth" "$ROOT_DIR/dist/$NAME"; do
    if [[ -f "$candidate" ]]; then BINARY="$candidate"; break; fi
  done
fi
if [[ -z "$BINARY" && "$ACTION" != install && "$ACTION" != update ]]; then
  [[ ! -x /usr/local/lib/campus-auth/campus-auth ]] || BINARY=/usr/local/lib/campus-auth/campus-auth
fi
TEMP_DIR=
trap '[[ -z "${TEMP_DIR:-}" ]] || rm -rf -- "$TEMP_DIR"' EXIT
if [[ -z "$BINARY" ]]; then
  [[ "$ACTION" != uninstall && "$ACTION" != rollback ]] || { echo '未找到已安装程序，请使用 --binary 指定原程序。' >&2; exit 2; }
  command -v curl >/dev/null || { echo '在线下载需要系统 curl；离线安装可直接使用 --binary 指定已下载程序。' >&2; exit 2; }
  command -v sha256sum >/dev/null || { echo '无法校验发行包：系统缺少 sha256sum。' >&2; exit 2; }
  TEMP_DIR="$(mktemp -d -t campus-auth-install.XXXXXXXX)"
  printf '正在下载 %s 并校验完整性……\n' "$NAME"
  curl --fail --show-error --silent --location --proto '=https' --tlsv1.2 "$BASE_URL/$NAME" -o "$TEMP_DIR/$NAME"
  curl --fail --show-error --silent --location --proto '=https' --tlsv1.2 "$BASE_URL/SHA256SUMS" -o "$TEMP_DIR/SHA256SUMS"
  CHECKSUM="$(awk -v name="$NAME" '$2 == name || $2 == "*" name {print $1 "  " name}' "$TEMP_DIR/SHA256SUMS")"
  [[ "$CHECKSUM" =~ ^[0-9a-fA-F]{64}[[:space:]] ]] || { echo '发行包缺少有效的 SHA256 校验值。' >&2; exit 2; }
  (cd -- "$TEMP_DIR" && printf '%s\n' "$CHECKSUM" | sha256sum --check --status) || { echo '发行包校验失败，已停止安装。' >&2; exit 2; }
  chmod 755 "$TEMP_DIR/$NAME"
  BINARY="$TEMP_DIR/$NAME"
fi
BINARY="$(realpath -- "$BINARY")"
[[ -f "$BINARY" ]] || { echo '找不到 CampusAuth 程序。' >&2; exit 2; }
[[ -x "$BINARY" ]] || chmod u+x "$BINARY"
if [[ "$ACTION" == tui ]]; then
  "$BINARY" tui "${ARGS[@]}"
elif [[ "$PREVIEW" == 1 ]]; then
  "$BINARY" service "$ACTION" --components "$COMPONENTS" "${ARGS[@]}"
elif [[ "$EUID" == 0 ]]; then
  "$BINARY" service "$ACTION" --components "$COMPONENTS" --apply "${ARGS[@]}"
else
  command -v sudo >/dev/null || { echo '安装系统服务需要 root 权限，请使用 root 运行此安装器。' >&2; exit 2; }
  sudo -- "$BINARY" service "$ACTION" --components "$COMPONENTS" --apply "${ARGS[@]}"
fi
