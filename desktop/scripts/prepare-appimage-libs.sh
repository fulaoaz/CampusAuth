#!/usr/bin/env bash
set -euo pipefail
desktop_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
destination="$desktop_dir/resources/linux"
mkdir -p "$destination"
# linuxdeploy excludes these as common system libraries, but our offline
# desktop package must also run on a baseline without GTK text dependencies.
for name in libfribidi.so.0 libharfbuzz.so.0 libgraphite2.so.3; do
  source="$(ldconfig -p | awk -v name="$name" '$1 == name && /x86-64/ && !found {print $NF; found=1}')"
  if [[ -z "$source" || ! -f "$source" ]]; then
    printf '构建机缺少 %s，请安装对应开发运行库后重新打包。\n' "$name" >&2
    exit 2
  fi
  cp -- "$(readlink -f -- "$source")" "$destination/$name"
done
for package in libfribidi0 libharfbuzz0b libgraphite2-3; do
  license="/usr/share/doc/$package/copyright"
  if [[ ! -f "$license" ]]; then
    printf '构建机缺少 %s 的版权声明，停止生成完整包。\n' "$package" >&2
    exit 2
  fi
  cp -- "$license" "$destination/$package-copyright"
done
