Backup Server — agent: accepts tasks from the client, runs backups, serves files over HTTPS.

Runs as a long-lived service with TLS (WSS + HTTPS download). Linux (VPS) and Windows.

Features:
  - WebSocket API (auth, sync_tasks, run_task, check_download)
  - mysql_dump / pg_dump / sqlite / folder archive / shell scripts
  - ZIP archives — built-in Rust library (no external zip command)
  - TLS required (server will not start without tls/server.crt and tls/server.key)
  - Task queue, one client per server
  - Temp files removed after client check_download

Quick start (Linux VPS):
  1. cargo build --release
  2. Place: backup-server, config.toml, tls/ (from client data/ca/{slug}/)
  3. chmod +x backup-server
  4. ./backup-server
  5. sudo ./service.sh   — systemd (from bundle)

Quick start (Windows):
  1. cargo build --release
  2. In one folder (e.g. C:\backup-server\):
       backup-server.exe
       config.toml
       tls\server.crt  and  tls\server.key
       data\temp\      (created automatically)
     config + tls from client data\ca\{slug}\.
  3. From PowerShell (use .\ prefix):
       cd C:\backup-server
       .\backup-server.exe
     Or double-click run-server.bat (next to exe).
  4. mysqldump_path in config.toml — full path or PATH
  5. Shell tasks: .bat / .cmd (not .sh)

Files:
  config.toml        — public_ip, port, debug, mysqldump_path, encrypt_password
  tls/server.crt     — certificate (from client)
  tls/server.key     — private key (on Linux: chmod 600)
  data/temp/         — temporary archives
  data/scripts/      — scripts for shell tasks

Linux service:
  systemctl status backup-server
  journalctl -u backup-server -f

License & disclaimer
  MIT License — see LICENSE at repository root.
  This software is a toolkit; data security is the administrator's responsibility.
  The author is not liable for loss, leakage, or corruption of your data.
  Details: client/docs/manual.html → License & disclaimer.
  Official builds: https://github.com/geniden/backup
