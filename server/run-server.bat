@echo off
cd /d "%~dp0"
if not exist "config.toml" (
    echo ERROR: config.toml not found in %CD%
    pause
    exit /b 1
)
if not exist "tls\server.crt" (
    echo ERROR: tls\server.crt not found. Copy tls\ from the client bundle.
    pause
    exit /b 1
)
echo Starting backup-server from %CD%
echo.
backup-server.exe
if errorlevel 1 pause
