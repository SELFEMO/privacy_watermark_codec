#!/usr/bin/env sh
set -eu

check() {
  if command -v "$1" >/dev/null 2>&1; then
    printf "[OK] %-10s %s\n" "$1" "$(command -v "$1")"
  else
    printf "[MISSING] %s\n" "$1"
  fi
}

check node
check npm
check rustc
check cargo
check ffmpeg
check ffprobe
