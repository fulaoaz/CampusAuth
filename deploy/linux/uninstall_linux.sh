#!/usr/bin/env bash
set -euo pipefail
# 默认保留账号与设置；明确使用 --purge 才清除已知 CampusAuth 数据。
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec bash "$SCRIPT_DIR/install_linux.sh" --action uninstall "$@"
