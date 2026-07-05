#!/bin/bash
# Sync server sources from Windows checkout → WSL build dir → musl release binary.
#
# Usage (from WSL):
#   chmod +x /mnt/c/rust/backup/server/build.sh
#   cd ~/backup-server-wsl && /mnt/c/rust/backup/server/build.sh
#
#   ./build.sh           # sync + incremental cargo build
#   ./build.sh -f        # sync + cargo clean + full rebuild
#
# Deploy to VPS (binary must match sha256 printed below):
#   scp target/x86_64-unknown-linux-musl/release/backup-server root@VPS:/path/backup-server

set -euo pipefail

WSL_BUILD_DIR="${WSL_BUILD_DIR:-$HOME/backup-server-wsl}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_DIR="${SOURCE_DIR:-$SCRIPT_DIR}"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
FORCE=0

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \?//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force) FORCE=1; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

OUT="$WSL_BUILD_DIR/target/$TARGET/release/backup-server"

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
    --exclude="files/" \
    --exclude="logs/" \
    --exclude="scripts/" \
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
    rm -f "$OUT"
fi

echo
echo "=== 3/4 cargo build --release --target $TARGET ==="
# Capture timing: if this finishes in ~1s, cargo reused artifacts (sources unchanged).
SECONDS=0
cargo build --release --target "$TARGET"
BUILD_SEC=$SECONDS

if [[ ! -f "$OUT" ]]; then
    echo "ERROR: binary not found: $OUT"
    exit 1
fi

mkdir -p "$WSL_BUILD_DIR/binaries"
cp -f "$OUT" "$WSL_BUILD_DIR/binaries/backup-server"
chmod +x "$OUT" "$WSL_BUILD_DIR/binaries/backup-server"

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
echo "Deploy to VPS and verify the SAME sha256 on the server:"
echo "  scp \"$OUT\" root@YOUR_VPS:/root/backup-server/backup-server"
echo "  ssh root@YOUR_VPS 'chmod +x backup-server && sha256sum backup-server && file backup-server'"
