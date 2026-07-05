# Publishing a GitHub Release

Portable **zip** (Windows) + **tar.gz** (Linux). One GitHub Release **v1.0.0** with all assets.

Repository: https://github.com/geniden/backup

## Build (same commit, version 1.0.0 in all Cargo.toml)

### Windows → `dist/win64/`

```powershell
cd C:\path\to\backup
.\build-all.ps1
# .\build-all.ps1 -Clean   # full rebuild
```

### Linux musl (WSL) → `dist/linux-musl/`

```bash
cd /path/to/backup
chmod +x ./build-all.sh
./build-all.sh -f
```

Per crate only:

```bash
cd ~/backup-decrypt-wsl && /path/to/backup/decrypt/build.sh -f
```

## Suggested release archives

| Archive | Contents |
|---------|----------|
| `backup-client-1.0.0-windows-x64.zip` | `backup-client.exe`, `backup-monitor.exe`, `client/README.txt` |
| `backup-client-1.0.0-linux-x64-musl.tar.gz` | `backup-client`, `backup-monitor` |
| `backup-server-1.0.0-windows-x64.zip` | `backup-server.exe`, `server/README.txt` |
| `backup-server-1.0.0-linux-x64-musl.tar.gz` | `backup-server` |
| `backup-decrypt-1.0.0-windows-x64.zip` | contents of `dist/win64/decrypt/` after `build-all.ps1` |
| `backup-decrypt-1.0.0-linux-x64.tar.gz` | `backup-decrypt`, same docs |

Do **not** ship `backup.db`, `decrypt.toml`, `data/ca/*.key`, or `config.toml` with real IPs/passwords.

## Git push + tag

```powershell
git remote add origin https://github.com/geniden/backup.git   # first time only
git add LICENSE README.md PAGE.md .gitignore client server decrypt docs
git status
git commit -m "Backup System v1.0.0"
git tag -a v1.0.0 -m "Backup System v1.0.0"
git push -u origin main
git push origin v1.0.0
gh release create v1.0.0 --repo geniden/backup --title "Backup System v1.0.0" --notes "See client/docs/manual.html and PAGE.md"
```

Attach zip/tar.gz files to the release on GitHub.
