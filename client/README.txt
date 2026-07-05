Backup Client — backup scheduler for your VPS servers.
Connects to backup-server over WSS (TLS), runs cron tasks, and downloads archives locally.

Features:
  - Multiple connections (one server = one connection)
  - Own CA and TLS certificates per server
  - Tasks: mysql_dump, postgresql_dump, sqlite_dump, files_archive, shell
  - Cron schedule, manual runs, scheduler daemon
  - SHA256 verification on download, temp cleanup on server

Quick start:
  1. cargo build --release
  2. ./backup-client          — interactive menu
  3. Add connection           — enter host:port (e.g. 203.0.113.10:8080), get data/ca/{slug}/
  4. Copy data/ca/{slug}/ to VPS (next to backup-server)
  5. Add task → Start scheduler

Files:
  backup.db          — connections, tasks, run history (SQLite WAL)
  data/ca/           — CA and deploy bundles for servers
  data/backups/      — downloaded archives
  data/backup.log    — client log

CLI:
  backup-client serve | connection-add | connection-test | task-add | task-list

Monitor (separate binary, read-only backup.db):
  backup-monitor                  — TUI alongside backup-client
  backup-monitor --db /path/to/backup.db

  Run while scheduler is active (second terminal or SSH session).
  Scheduler logs: tail -f data/backup.log

Build:
  cargo build --release           — backup-client + backup-monitor
  cargo build --release --no-default-features  — backup-client only (no TUI)

License & disclaimer
  MIT License — see LICENSE at repository root.
  This software is a toolkit; data security is the administrator's responsibility.
  The author is not liable for loss, leakage, or corruption of your data.
  Details: client/docs/manual.html → License & disclaimer.
  Official builds: https://github.com/geniden/backup
