#!/usr/bin/env sh
set -eu
if [ "$#" -lt 1 ]; then
  printf '%s\n' '用法：run-appimage.sh CampusAuth.AppImage [应用参数...]' >&2
  exit 2
fi
image=$1
shift
case "$image" in
  /*) ;;
  *) image="$(pwd)/$image" ;;
esac
if [ ! -f "$image" ]; then
  printf '%s\n' '找不到 CampusAuth AppImage 文件。' >&2
  exit 2
fi
# AppImage runtime performs extraction without loading FUSE. No installation or
# elevated permissions are needed; the temporary extracted tree is cleaned up
# by the AppImage runtime after the application exits.
APPIMAGE_EXTRACT_AND_RUN=1 "$image" "$@"
