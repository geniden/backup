# Backup System — setup in one evening

Friendly walkthrough: mini PC + VPS + first backup · v1.0.0

**~15 min to first backup** · Windows client · Linux server · optional USB decrypt

Full reference: [manual.html](../client/docs/manual.html) · [PAGE.md](../PAGE.md) · [Releases](https://github.com/geniden/backup/releases)

**Русская версия:** [TUTORIAL.ru.md](TUTORIAL.ru.md) · Offline HTML: [tutorial.en.html](tutorial.en.html)

---

## Contents

1. [Why I do it this way](#why-i-do-it-this-way)
2. [Mini PC & downloads](#mini-pc--downloads)
3. [Part 1 — client & server](#part-1--client--server)
4. [Part 2 — tasks & scheduler](#part-2--tasks--scheduler)
5. [Part 3 — production & encryption](#part-3--production--encryption)
6. [Monitoring](#monitoring)
7. [Troubleshooting](#troubleshooting)

---

## Why I do it this way

I run backups for small business sites — SaaS and shops. Full disk images are too heavy; a MySQL dump plus a folder archive is usually enough.

You *can* run backup-client on a VPS, but I keep archives on a **dedicated machine** at home.

- Backups on a quiet mini PC (~35 W)
- Own `device_id` on the backup PC
- In Production secrets mode, DB passwords stay on the server
- AES archives + decrypt keys on a USB stick

This guide is not the full manual — every field has hints in the apps. Here we cover the happy path and what the menus look like.

### Five protection layers

1. **TLS** — private CA, WSS + HTTPS
2. **Encrypt mode** — AES on server → `*.aes` on disk
3. **device_id** — server accepts only your backup PC
4. **Secrets: Production** — DB passwords on backup-server only
5. **backup-decrypt on USB** — decrypt passwords off the scheduler PC

---

## Mini PC & downloads

### Hardware

**HP ProDesk 260 G2** (used, ~$50–75): i3-6100, 16 GB RAM, SSD 256 GB, silent, ~35 W.

![24/7 backup-client on a shelf](screenshots/mini-pc.jpg)

### Download

[GitHub Releases](https://github.com/geniden/backup/releases):

- Mini PC (Windows): `backup-client-1.0.0-windows-x64.zip`
- VPS (Linux): `backup-server-1.0.0-linux-x64-musl.tar.gz`
- USB (optional): `backup-decrypt-1.0.0-windows-x64.zip`

Unpack under `C:\backup\`:

![C:\backup\ layout](screenshots/client-0.jpg)

![backup-client.exe](screenshots/client-1.jpg)

![backup-monitor (optional)](screenshots/monitor-1.jpg)

![backup-server on Linux VPS](screenshots/server-2.jpg)

![backup-decrypt on USB](screenshots/decrypt-1.jpg)

---

## Part 1 — client & server

### 1.1. First launch

Run `C:\backup\client\backup-client.exe`

Main menu → **Settings** → **Language** (English, Deutsch, Français, Русский, 中文).

### 1.2. Add connection

**Connections** → **Add connection**

On **Test connection now?** press **n** — the server is not up yet.

![Connection ID (slug) and host:port](screenshots/client-3.jpg)

```
C:\backup\client\data\ca\bash02\
├── config.toml
├── tls\
├── service.sh
└── README.txt
```

> **TLS** — your private CA. Folder `data\ca\bash02\` is the full deploy bundle for the VPS.

### 1.3. Copy to VPS

> **FileZilla warning:** in my experience it sometimes **corrupts** the Linux binary (`Exec format error`). Prefer **WinSCP (Binary)**, **FAR + SFTP**, or **scp**. After copy: `file backup-server` → `ELF 64-bit … statically linked`.

```bash
mkdir -p /opt/backup
```

```powershell
scp -r C:\backup\client\data\ca\bash02\* user@203.0.113.10:/opt/backup/
scp C:\backup\server\backup-server user@203.0.113.10:/opt/backup/
```

![Expected contents of /opt/backup/](screenshots/server-1.jpg)

### 1.4. Start on VPS

```bash
cd /opt/backup
chmod +x backup-server service.sh
./backup-server
```

`chmod 600 tls/server.key` is usually **not required** — backup-server and `service.sh` set it automatically. Use manually only if the server complains about key permissions.

![Trial run or systemctl status](screenshots/server-2.jpg)

```bash
sudo ./service.sh
sudo systemctl status backup-server
```

#### systemctl cheat sheet

| Command | Action |
|---------|--------|
| `sudo systemctl start backup-server` | Start |
| `sudo systemctl stop backup-server` | Stop |
| `sudo systemctl restart backup-server` | After editing config.toml |
| `sudo systemctl enable backup-server` | Start on boot |
| `sudo journalctl -u backup-server -f` | Live logs |

```bash
free -h
df -h
uptime
```

### 1.5. Test from client

**Connections** → `bash02` → **Test connection**

On first success the server stores this PC’s `device_id` in `config.toml`.

![Connection OK — TLS + device_id registered](screenshots/client-6.jpg)

Move to a new PC: clear `device_id = ""` on the server → `systemctl restart backup-server` → **Test connection** from the new machine.

---

## Part 2 — tasks & scheduler

### 2.1. Add task

**Connections** → `bash02` → **Add task**

Task setup is the core of the system: type, parameters, and **schedule** define what is copied from the VPS and when. Everything else (scheduler, encryption) builds on tasks you create here.

![Tasks menu](screenshots/client-4.jpg)

| Type | Use when |
|------|----------|
| mysql_dump | MySQL / MariaDB |
| postgresql_dump | PostgreSQL |
| files_archive | Site folder, configs |
| sqlite_dump | SQLite file |
| shell | Script in data/scripts/ |
| dir_sync | Incremental sync (no AES) |

![Task name, schedule, DB or path settings](screenshots/client-5.jpg)

#### Schedule (cron)

When prompted, the client shows field help. After you confirm, you see the full expression, e.g. `→ saved as: 0 8 * * *`.

The format has **5 fields**: minute · hour · day-of-month · month · day-of-week. You do **not** have to type asterisks `*` (“every”) — missing fields are filled in automatically. A value like `/N` is treated as `*/N` (every N in that field).

| What you type | Saved as | When it runs |
|---------------|----------|--------------|
| `*/5` | `*/5 * * * *` | every 5 minutes (for testing) |
| `0 8` | `0 8 * * *` | daily at 08:00 |
| `0 2` | `0 2 * * *` | daily at 02:00 |
| `0 /2` | `0 */2 * * *` | every 2 hours (at :00) |
| `*/60` or `/60` | `0 * * * *` | every hour |
| `*/120` | `0 */2 * * *` | every 2 hours |
| `*/30 * * * *` | — | every 30 minutes |
| `0 0 * * 0` | — | every Sunday at midnight |

> **Many tasks at the same time:** If several tasks (5–10 or more) start in the **same minute**, the server may not queue them all in time — some runs are **SKIPPED** (log: “SKIPPED this run: not queued after … attempts”). Stagger tasks by **at least one minute**: first at minute `0`, second at `1`, third at `2`… Or use 5-minute steps: `0`, `5`, `10` in the same hour. Example for three DB dumps in the morning: `0 8 * * *`, `1 8 * * *`, `2 8 * * *`.

#### Folder paths on the VPS

Only **files_archive** and **dir_sync** read files on the server directly — the task needs an **absolute path** to a directory. Other types (DB dumps, sqlite, shell) use their own fields (DB host, script path, etc.).

To get the path on the VPS for a folder, SSH in and run:

```bash
cd /var/www/site && pwd
```

Use the output (e.g. `/var/www/site`) in the path field when creating the task.

### 2.2. Run now

Open the task → **Run now (manual test)**

![Run now](screenshots/client-7.jpg)

```
C:\backup\client\data\backups\bash02\
  backup_....zip
```

### 2.3. Scheduler 24/7

Main menu → **Start scheduler** or `backup-client.exe serve`

![Main menu → Start scheduler](screenshots/client-9.jpg)

![Scheduler running](screenshots/client-12.jpg)

![File in data\backups\](screenshots/client-11-result.jpg)

### 2.4. Windows autostart

**Win+R** → `taskschd.msc` → Create Task:

- Program: `C:\backup\client\backup-client.exe`
- Arguments: `serve`
- Start in: `C:\backup\client`
- Trigger: At startup or At log on

Simpler (after logon only): `shell:startup` → shortcut with `serve`.

```powershell
Get-Content C:\backup\client\data\backup.log -Wait -Tail 30
```

---

## Part 3 — production & encryption

### 3.1. Secrets mode: Production

When everything works: connection menu → switch **Secrets mode: Production (passwords on server)**, then **Test connection**.

### 3.2. Encrypt mode

**How encrypt_password works**

- Each **connection** (each VPS) can have its own `encrypt_password` in that server’s `config.toml`. backup-client only toggles **Encrypt mode (on)** per task — the password is *not* stored on the backup PC.
- Only **backup-server** (encrypts before download) and whoever created the password know it. You can choose your own passphrase or generate a random one in **backup-decrypt** and paste the same value into the server config.

`backup-decrypt.exe` → **List connections** → **Add connection** → name `bash02` (same slug) → copy the shown password.

![backup-decrypt: Add connection](screenshots/decrypt-1.jpg)

![Copy password to the server](screenshots/decrypt-2.jpg)

On VPS `/opt/backup/config.toml`:

```toml
encrypt_password = "same_password_here"
```

```bash
sudo systemctl restart backup-server
```

In backup-client: task menu → **Encrypt mode (on)** → downloads are `*.zip.aes`.

![Browse .aes files — set backups root in Settings](screenshots/decrypt-3.jpg)

> Keep **decrypt.toml** on USB, not on the same mini PC as the scheduler.

---

## Monitoring

- Files under `data\backups\bash02\`
- Client log without ERROR/WARN
- `backup-monitor.exe` for a live dashboard

![Monitor](screenshots/monitor-1.jpg)

---

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Corrupted binary / FileZilla | WinSCP Binary, FAR, scp; `file backup-server` |
| device_id / wrong PC | `device_id = ""` on server, restart |
| mysqldump not found | `which mysqldump` → server config.toml |
| Encrypt task skipped | Empty encrypt_password on server |
| No file on client | Is `serve` / Task Scheduler running? |

Details: [manual.html](../client/docs/manual.html)

---

Backup System v1.0.0 · [github.com/geniden/backup](https://github.com/geniden/backup)

Menu labels match backup-client `en.json`. Offline HTML: [tutorial.en.html](tutorial.en.html)
