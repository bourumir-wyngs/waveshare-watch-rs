#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${1:-/dev/ttyACM0}"
BIN="$ROOT_DIR/target/xtensa-esp32s3-none-elf/release/waveshare-watch-rs"

cd "$ROOT_DIR"

if [[ -f "$HOME/export-esp.sh" ]]; then
    # Provides xtensa-esp32s3-elf-gcc and ESP Rust toolchain environment.
    # shellcheck disable=SC1090
    . "$HOME/export-esp.sh"
fi

command -v cargo >/dev/null || {
    echo "cargo not found in PATH" >&2
    exit 1
}

command -v espflash >/dev/null || {
    echo "espflash not found in PATH" >&2
    exit 1
}

if [[ ! -e "$PORT" ]]; then
    echo "Serial port not found: $PORT" >&2
    echo "Usage: ./reflash.sh [/dev/ttyACM0]" >&2
    exit 1
fi

if command -v fuser >/dev/null && fuser "$PORT" >/dev/null 2>&1; then
    echo "Serial port is busy: $PORT" >&2
    fuser -v "$PORT" >&2 || true
    echo "Stop the process using it, then rerun this script." >&2
    exit 1
fi

cargo build --release
espflash flash --port "$PORT" "$BIN"
