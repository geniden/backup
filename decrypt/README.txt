backup-decrypt — decrypt .aes backup files from backup-server
=============================================================

Format: BACKUPENC1 + AES-256-GCM (same key derivation as server: SHA256(password)).
Password can be any length (passphrase, words, random string).

Recommended: run from a USB stick — keep decrypt.toml with passwords off the backup PC.

Setup
-----
1. Copy backup-decrypt and decrypt.toml to USB (or any folder).
2. Use the app menu (List connections → Add connection) or edit decrypt.toml:
     path = "E:/backups"              # root; inside: production/, staging/, ...
     output_path = ""                 # empty = next to .aes

     [profiles]
     production = "same as encrypt_password on that VPS"
     staging    = "password for second server"
3. On each server: Encrypt mode on tasks + encrypt_password in config.toml.

Folder layout (matches backup-client downloads):
  {path}/{slug}/backup_task_mysql_2026-07-05_120000.zip.aes

Profiles (connection names)
---------------------------
  Connection name = slug in backup-client = subfolder under path (not the task name in the filename).
  Browser auto-picks password from folder path (…/production/file.aes → connection production).
  One connection only → used for all files when the path has no slug subfolder.

Output names
------------
  Encrypt off  →  backup_foo.zip  (or .txt for shell)
  Encrypt on   →  backup_foo.zip.aes  (or .txt.aes)

Run
---
  backup-decrypt                         # main menu
  backup-decrypt --key-file E:\decrypt.toml
  backup-decrypt E:\backups\production   # browser in subfolder
  backup-decrypt --file path\to\file.zip.aes
  backup-decrypt --file file.zip.aes --profile production

Main menu
---------
  - Browse .aes files
  - List connections (Add connection, edit per VPS)
  - Settings (backups root folder, output path)
  - Exit

List connections
----------------
  Add connection — enter name (same as backup-client slug); a random password is generated and shown.
  Copy it to encrypt_password on that server.
  Each saved connection: Change name / Change password / Regenerate password / Delete.

Settings
--------
  Set backups root folder — e.g. E:/backups or copy of client data/backups
  Set output path — optional folder for plaintext (empty = next to .aes)

Browser
-------
  .. (back)     — parent folder (stops at root)
  [DIR]  name/  — enter subfolder (slug)
  [AES]  file   — decrypt (auto connection or pick from list)
  Wrong password → try another connection

Do not commit decrypt.toml to git — it contains secrets.

License & disclaimer
--------------------
  MIT License — see LICENSE at repository root.
  This software is a toolkit; data security is the administrator's responsibility.
  The author is not liable for loss, leakage, or corruption of your data.
  Details: client/docs/manual.html → License & disclaimer.
  Official builds: https://github.com/geniden/backup
