# Build from source

Requires **[Rust 1.70+](https://rust.rust-lang.org/)** (latest stable recommended).  
`edition = "2021"` in `Cargo.toml` is the **Rust language edition**, not a calendar year.

**Pre-built binaries:** [GitHub Releases](https://github.com/geniden/backup/releases) — most users do not need to compile.

---

## Windows

Run **`build-all.ps1`** in the repo root → binaries in **`dist/win64/`** (see script). Or per crate:

```powershell
cd client
cargo build --release
# → target\release\backup-client.exe, backup-monitor.exe

cd ..\server
cargo build --release
# → target\release\backup-server.exe

cd ..\decrypt
cargo build --release
# → target\release\backup-decrypt.exe
```

---

## Linux (musl, portable static binaries)

From **WSL**, one command (adjust the repo path to your clone):

```bash
chmod +x /path/to/backup/build-all.sh
/path/to/backup/build-all.sh -f
```

Or per crate:

```bash
cd ~/backup-client-wsl && /path/to/backup/client/build.sh -f
cd ~/backup-server-wsl && /path/to/backup/server/build.sh -f
cd ~/backup-decrypt-wsl && /path/to/backup/decrypt/build.sh -f
```

---

## Verify

```bash
backup-client --version
backup-server   # prints banner with version
```

---

## Publishing releases (maintainers)

See [docs/RELEASE.md](docs/RELEASE.md) for zip/tar.gz assets and creating a GitHub Release.
