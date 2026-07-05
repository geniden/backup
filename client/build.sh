#!/bin/bash
# Sync client sources from Windows checkout → WSL build dir → musl release binaries.
#
# Usage (from WSL):
#   chmod +x /mnt/c/rust/backup/client/build.sh
#   cd ~/backup-client-wsl && /mnt/c/rust/backup/client/build.sh
#
#   ./build.sh           # sync + incremental cargo build
#   ./build.sh -f        # sync + cargo clean + full rebuild
#
# Windows release (icons embedded): build on Windows, not in WSL:
#   cd C:\rust\backup\client
#   cargo build --release
#   → target\release\backup-client.exe, backup-monitor.exe
#
# Deploy Linux bundle (verify sha256 printed below):
#   scp target/x86_64-unknown-linux-musl/release/backup-client root@HOST:/opt/backup-client/
#   scp target/x86_64-unknown-linux-musl/release/backup-monitor root@HOST:/opt/backup-client/

set -euo pipefail

WSL_BUILD_DIR="${WSL_BUILD_DIR:-$HOME/backup-client-wsl}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_DIR="${SOURCE_DIR:-$SCRIPT_DIR}"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
FORCE=0

usage() {
    sed -n '2,18p' "$0" | sed 's/^# \?//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force) FORCE=1; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

OUT_DIR="$WSL_BUILD_DIR/target/$TARGET/release"
OUT_CLIENT="$OUT_DIR/backup-client"
OUT_MONITOR="$OUT_DIR/backup-monitor"

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
    --exclude="data/" \
    --exclude="backup.db" \
    --exclude="backup.db-*" \
    "$SOURCE_DIR/" "$WSL_BUILD_DIR/"

echo
echo "Top-level src:"
ls -1 "$WSL_BUILD_DIR/src/"*.rs 2>/dev/null | head -20 || true
echo "Subdirs:"
ls -1d "$WSL_BUILD_DIR/src/"*/ 2>/dev/null || true

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
    rm -f "$OUT_CLIENT" "$OUT_MONITOR"
fi

echo
echo "=== 3/4 cargo build --release --target $TARGET ==="
SECONDS=0
cargo build --release --target "$TARGET"
BUILD_SEC=$SECONDS

missing=0
for bin in "$OUT_CLIENT" "$OUT_MONITOR"; do
    if [[ ! -f "$bin" ]]; then
        echo "ERROR: binary not found: $bin"
        missing=1
    fi
done
if [[ $missing -eq 1 ]]; then
    exit 1
fi

mkdir -p "$WSL_BUILD_DIR/binaries"
cp -f "$OUT_CLIENT" "$WSL_BUILD_DIR/binaries/backup-client"
cp -f "$OUT_MONITOR" "$WSL_BUILD_DIR/binaries/backup-monitor"
chmod +x "$OUT_CLIENT" "$OUT_MONITOR" "$WSL_BUILD_DIR/binaries/"*

echo
echo "=== 4/4 Result (${BUILD_SEC}s) ==="
for bin in backup-client backup-monitor; do
    path="$OUT_DIR/$bin"
    echo "--- $bin ---"
    ls -l "$path"
    file "$path"
    HASH=$(sha256sum "$path" | awk '{print $1}')
    echo "sha256: $HASH"
    echo
done

ldd "$OUT_CLIENT" 2>&1 || true

if [[ $BUILD_SEC -lt 3 && $FORCE -eq 0 ]]; then
    echo
    echo "Note: build was very fast — Cargo did not recompile (sources unchanged since last build)."
    echo "      Binary mtime may stay old; that is normal. Use ./build.sh -f for a full rebuild."
fi

echo
echo "Linux deploy example:"
echo "  scp \"$OUT_CLIENT\" \"$OUT_MONITOR\" root@YOUR_HOST:/opt/backup-client/"
echo "  ssh root@YOUR_HOST 'chmod +x /opt/backup-client/backup-client /opt/backup-client/backup-monitor'"
