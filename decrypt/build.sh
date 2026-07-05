#!/bin/bash
# Sync decrypt sources from Windows checkout → WSL build dir → musl release binary.
#
# Usage (from WSL):
#   chmod +x /mnt/c/rust/backup/decrypt/build.sh
#   cd ~/backup-decrypt-wsl && /mnt/c/rust/backup/decrypt/build.sh
#
#   ./build.sh           # sync + incremental cargo build
#   ./build.sh -f        # sync + cargo clean + full rebuild
#
# Windows release (icon embedded): build on Windows, not in WSL:
#   cd C:\rust\backup\decrypt
#   cargo build --release
#   → target\release\backup-decrypt.exe
#
# Or use repo root: /mnt/c/rust/backup/build-all.sh

set -euo pipefail

WSL_BUILD_DIR="${WSL_BUILD_DIR:-$HOME/backup-decrypt-wsl}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_DIR="${SOURCE_DIR:-$SCRIPT_DIR}"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
FORCE=0

usage() {
    sed -n '2,16p' "$0" | sed 's/^# \?//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force) FORCE=1; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

OUT="$WSL_BUILD_DIR/target/$TARGET/release/backup-decrypt"

echo "Source:      $SOURCE_DIR"
echo "Build dir:   $WSL_BUILD_DIR"
echo "Rust target: $TARGET"
echo "Force rebuild: $([[ $FORCE -eq 1 ]] && echo yes || echo no)"
echo

if [[ ! -f "$SOURCE_DIR/Cargo.toml" ]]; then
    echo "ERROR: $SOURCE_DIR/Cargo.toml not found"
    exit 1
fi

mkdir -p "$WSL_BUILD_DIR"

echo "=== 1/4 Sync sources (checksum; deletes stale files in WSL copy) ==="
rsync -av --delete --checksum \
    --exclude=".git/" \
    --exclude="target/" \
    --exclude="decrypt.toml" \
    "$SOURCE_DIR/" "$WSL_BUILD_DIR/"

echo
echo "Top-level src:"
ls -1 "$WSL_BUILD_DIR/src/"*.rs 2>/dev/null | head -20 || true

if ! rustup target list --installed | grep -q "^${TARGET}$"; then
    echo "Installing Rust target ${TARGET}..."
    rustup target add "$TARGET"
fi

if ! command -v musl-gcc >/dev/null 2>&1; then
    echo "Installing musl-tools (musl-gcc linker)..."
    sudo apt-get update && sudo apt-get install -y musl-tools
fi

cd "$WSL_BUILD_DIR"

if [[ $FORCE -eq 1 ]]; then
    echo
    echo "=== 2/4 Force: cargo clean ==="
    cargo clean
    rm -f "$OUT"
fi

echo
echo "=== 3/4 cargo build --release --target $TARGET ==="
SECONDS=0
cargo build --release --target "$TARGET"
BUILD_SEC=$SECONDS

if [[ ! -f "$OUT" ]]; then
    echo "ERROR: binary not found: $OUT"
    exit 1
fi

mkdir -p "$WSL_BUILD_DIR/binaries"
cp -f "$OUT" "$WSL_BUILD_DIR/binaries/backup-decrypt"
chmod +x "$OUT" "$WSL_BUILD_DIR/binaries/backup-decrypt"

echo
echo "=== 4/4 Result (${BUILD_SEC}s) ==="
ls -l "$OUT"
file "$OUT"
ldd "$OUT" 2>&1 || true
HASH=$(sha256sum "$OUT" | awk '{print $1}')
echo "sha256: $HASH"

if [[ $BUILD_SEC -lt 3 && $FORCE -eq 0 ]]; then
    echo
    echo "Note: build was very fast — Cargo did not recompile (sources unchanged since last build)."
    echo "      Binary mtime may stay old; that is normal. Use ./build.sh -f for a full rebuild."
fi

echo
echo "Copy to USB or dist: see repo build-all.sh"
