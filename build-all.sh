#!/bin/bash
# Build all Linux musl release binaries (WSL) and copy to Windows dist folder.
#
# Usage (from WSL, in repo root):
#   chmod +x ./build-all.sh
#   ./build-all.sh        # incremental
#   ./build-all.sh -f     # full rebuild (cargo clean in each crate)
#
# Output (under repo dist/):
#   dist/linux-musl/client/   backup-client, backup-monitor
#   dist/linux-musl/server/   backup-server
#   dist/linux-musl/decrypt/  backup-decrypt, decrypt.toml.example, README.txt
#
# Windows binaries: use build-all.ps1 → dist\win64\

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$SCRIPT_DIR}"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
FORCE=0

CLIENT_WSL="${CLIENT_WSL:-$HOME/backup-client-wsl}"
SERVER_WSL="${SERVER_WSL:-$HOME/backup-server-wsl}"
DECRYPT_WSL="${DECRYPT_WSL:-$HOME/backup-decrypt-wsl}"

DIST="$REPO_ROOT/dist/linux-musl"

usage() {
    sed -n '2,14p' "$0" | sed 's/^# \?//'
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -f|--force) FORCE=1; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

FORCE_ARG=()
[[ $FORCE -eq 1 ]] && FORCE_ARG=(-f)

echo "=============================================="
echo " Backup System — Linux musl build-all"
echo " Repo:   $REPO_ROOT"
echo " Dist:   $DIST"
echo " Target: $TARGET"
echo " Force:  $([[ $FORCE -eq 1 ]] && echo yes || echo no)"
echo "=============================================="
echo

for script in client/build.sh server/build.sh decrypt/build.sh; do
    path="$REPO_ROOT/$script"
    if [[ ! -x "$path" ]]; then
        chmod +x "$path"
    fi
done

echo ">>> client (backup-client + backup-monitor)"
WSL_BUILD_DIR="$CLIENT_WSL" "$REPO_ROOT/client/build.sh" "${FORCE_ARG[@]}"

echo
echo ">>> server (backup-server)"
WSL_BUILD_DIR="$SERVER_WSL" "$REPO_ROOT/server/build.sh" "${FORCE_ARG[@]}"

echo
echo ">>> decrypt (backup-decrypt)"
WSL_BUILD_DIR="$DECRYPT_WSL" "$REPO_ROOT/decrypt/build.sh" "${FORCE_ARG[@]}"

OUT_CLIENT="$CLIENT_WSL/target/$TARGET/release"
OUT_SERVER="$SERVER_WSL/target/$TARGET/release/backup-server"
OUT_DECRYPT="$DECRYPT_WSL/target/$TARGET/release/backup-decrypt"

mkdir -p "$DIST/client" "$DIST/server" "$DIST/decrypt"

cp -f "$OUT_CLIENT/backup-client" "$OUT_CLIENT/backup-monitor" "$DIST/client/"
cp -f "$REPO_ROOT/client/README.txt" "$DIST/client/"
chmod +x "$DIST/client/"*

cp -f "$OUT_SERVER" "$DIST/server/backup-server"
cp -f "$REPO_ROOT/server/README.txt" "$DIST/server/"
chmod +x "$DIST/server/backup-server"

cp -f "$OUT_DECRYPT" "$DIST/decrypt/backup-decrypt"
cp -f "$REPO_ROOT/decrypt/decrypt.toml.example" "$DIST/decrypt/"
cp -f "$REPO_ROOT/decrypt/README.txt" "$DIST/decrypt/"
chmod +x "$DIST/decrypt/backup-decrypt"

echo
echo "=============================================="
echo " Done — dist/linux-musl/"
echo "=============================================="
find "$DIST" -type f | sort | while read -r f; do
    rel="${f#$DIST/}"
    hash=$(sha256sum "$f" | awk '{print $1}')
    size=$(ls -lh "$f" | awk '{print $5}')
    printf "  %-45s %6s  sha256:%s...\n" "$rel" "$size" "${hash:0:16}"
done

echo
echo "Pack for GitHub Release:"
echo "  cd $DIST/client && tar -czvf ../../backup-client-1.0.0-linux-x64-musl.tar.gz backup-client backup-monitor README.txt"
echo "  cd $DIST/server && tar -czvf ../../backup-server-1.0.0-linux-x64-musl.tar.gz backup-server README.txt"
echo "  cd $DIST/decrypt && tar -czvf ../../backup-decrypt-1.0.0-linux-x64-musl.tar.gz backup-decrypt decrypt.toml.example README.txt"
