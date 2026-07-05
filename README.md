# Backup System

Distributed backup toolkit for your own VPS servers (**Windows** and **Linux**).  
A central **backup-client** schedules tasks over encrypted WSS, downloads archives, and stores them locally. Each VPS runs a lightweight **backup-server** agent. Optional **backup-decrypt** on USB decrypts `.aes` files offline.

**Author:** Emelyanov Anton · [geniden@gmail.com](mailto:geniden@gmail.com) · [github.com/geniden/backup](https://github.com/geniden/backup)  
**License:** [MIT](LICENSE) · **Version:** 1.0.0

> **Official pre-built binaries:** download from [GitHub Releases](https://github.com/geniden/backup/releases).  
> You may also **build from source** for your own OS (e.g. macOS or a specific Linux) — see [Build from source](#build-from-source).

---

## Components

| Program | Role |
|---------|------|
| **backup-client** | Interactive setup, cron scheduler, downloads, SHA256 verification |
| **backup-monitor** | Read-only TUI dashboard over `backup.db` (optional) |
| **backup-server** | Task queue and execution on each VPS |
| **backup-decrypt** | Decrypt `*.aes` archives; keep passwords on USB, off the backup PC |

Full documentation: **[User Manual](client/docs/manual.html)** · **[PAGE.md](PAGE.md)** (quick start)

Per-component notes: [client/README.txt](client/README.txt) · [server/README.txt](server/README.txt) · [decrypt/README.txt](decrypt/README.txt)

---

## Quick start (overview)

1. Install **backup-client** (+ optional **backup-monitor**) on your backup machine.
2. Add a connection → client generates TLS bundle under `data/ca/{slug}/`.
3. Deploy **backup-server** + bundle to each VPS.
4. Add tasks, enable encryption if needed, start `backup-client serve`.
5. Optionally keep **backup-decrypt** + `decrypt.toml` on a USB stick.

Details: [manual — deployment](client/docs/manual.html#deployment).

---

## Build from source

Requires **[Rust 1.70+](https://rust.rust-lang.org/)** (latest stable recommended). Crates use language edition 2021 (standard; not a calendar year).

Windows: run **`build-all.ps1`** in the repo root → binaries in **`dist/win64/`** (see script). Or manually:

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

### Linux (musl, portable static binaries)

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

Check version: `backup-client --version`, `backup-server` banner, etc.

---

## Repository layout

```
client/     backup-client, backup-monitor, manual, locales
server/     backup-server agent
decrypt/    offline decrypt tool (backup-decrypt)
LICENSE     MIT
```

Each app is a separate Rust crate with its own `Cargo.toml`. First run creates local data (`backup.db`, `data/` on client; `config.toml`, `backup.db` on server) — these are **not** part of the repository.

---

## Security model (summary)

The software is a **toolkit for building your own backup security stack** — TLS, optional server-side AES encryption, hardware-bound client, secrets on server, offline decrypt. **Which layers you enable and how you operate them is your choice.**

Security depends on the **system administrator**: root CA key, firewall, `encrypt_password`, server `backup.db`, USB with decrypt passwords, VPS hardening, and using **official builds** from this repository.

---

## Disclaimer

THE SOFTWARE IS PROVIDED **"AS IS"** UNDER THE [MIT LICENSE](LICENSE).

The author is **not responsible** for loss, corruption, leakage, unauthorized access, or unavailability of your data — including from misconfiguration, stolen keys, hardware failure, unofficial builds, or any other cause.

**Data security is the responsibility of the person who deploys and operates the software.**

Expanded text: [manual — License & disclaimer](client/docs/manual.html#license).

---

## Publishing releases (maintainers)

See [docs/RELEASE.md](docs/RELEASE.md) for building zip/tar.gz assets and creating a GitHub Release.
