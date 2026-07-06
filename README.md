# Backup System

<p align="center">
  <img src="client/docs/art512.jpg" alt="Backup System — layered protection" width="512">
</p>

Distributed backup toolkit for your own VPS servers (**Windows** and **Linux**).  
A central **backup-client** schedules tasks over encrypted WSS, downloads archives, and stores them locally. Each VPS runs a lightweight **backup-server** agent. Optional **backup-decrypt** on USB decrypts `.aes` files offline.

**Author:** Emelyanov Anton · [geniden@gmail.com](mailto:geniden@gmail.com) · [github.com/geniden/backup](https://github.com/geniden/backup)  
**License:** [MIT](LICENSE) · **Version:** 1.0.0

---

## Get started

1. **Download** pre-built binaries from **[GitHub Releases](https://github.com/geniden/backup/releases)**  
   (`backup-client` + optional `backup-monitor` on your PC; `backup-server` on each VPS; `backup-decrypt` on USB if you use encryption).

2. **Follow the step-by-step guide** (screenshots, tasks, scheduler, encryption):  
   **[Setup guide (English)](docs/TUTORIAL.en.md)** · **[Инструкция (русский)](docs/TUTORIAL.ru.md)**  
   Readable directly on GitHub. Offline HTML copies: [EN](docs/tutorial.en.html) · [RU](docs/tutorial.ru.html).

3. **Need every setting explained?** See the **[User Manual](client/docs/manual.html)** (full reference; clone and open in a browser).

---

## Components

| Program | Role |
|---------|------|
| **backup-client** | Interactive setup, cron scheduler, downloads, SHA256 verification |
| **backup-monitor** | Read-only TUI dashboard over `backup.db` (optional) |
| **backup-server** | Task queue and execution on each VPS |
| **backup-decrypt** | Decrypt `*.aes` archives; keep passwords on USB, off the backup PC |

Per-component notes: [client/README.txt](client/README.txt) · [server/README.txt](server/README.txt) · [decrypt/README.txt](decrypt/README.txt)

---

## Documentation map

| Document | Purpose |
|----------|---------|
| **[docs/TUTORIAL.en.md](docs/TUTORIAL.en.md)** | **Main onboarding (EN)** — read on GitHub |
| **[docs/TUTORIAL.ru.md](docs/TUTORIAL.ru.md)** | Same guide in Russian |
| [docs/tutorial.en.html](docs/tutorial.en.html) · [docs/tutorial.ru.html](docs/tutorial.ru.html) | Offline HTML (open in browser after clone) |
| **[client/docs/manual.html](client/docs/manual.html)** | **Full reference** — all menus, fields, security layers |
| **[PAGE.md](PAGE.md)** | Short markdown overview (security layers, quick checklist) |
| **[BUILD.md](BUILD.md)** | Build from source (developers) |

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
