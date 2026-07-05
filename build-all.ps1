# Build all Windows release binaries into dist/win64/
#
# Usage (PowerShell, from repo root):
#   .\build-all.ps1
#   .\build-all.ps1 -Clean
#
# Output:
#   dist\win64\client\backup-client.exe, backup-monitor.exe
#   dist\win64\server\backup-server.exe
#   dist\win64\decrypt\backup-decrypt.exe, decrypt.toml.example, README.txt

param(
    [switch]$Clean
)

$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot

$DistClient = Join-Path $Root "dist\win64\client"
$DistServer = Join-Path $Root "dist\win64\server"
$DistDecrypt = Join-Path $Root "dist\win64\decrypt"

function Build-Crate {
    param(
        [string]$Dir,
        [string]$Label
    )
    Write-Host ""
    Write-Host "=== $Label ===" -ForegroundColor Cyan
    Push-Location (Join-Path $Root $Dir)
    try {
        if ($Clean) {
            cargo clean
        }
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed in $Dir" }
    }
    finally {
        Pop-Location
    }
}

New-Item -ItemType Directory -Force -Path $DistClient, $DistServer, $DistDecrypt | Out-Null

Build-Crate "client" "backup-client + backup-monitor"
Build-Crate "server" "backup-server"
Build-Crate "decrypt" "backup-decrypt"

Copy-Item (Join-Path $Root "client\target\release\backup-client.exe") $DistClient -Force
Copy-Item (Join-Path $Root "client\target\release\backup-monitor.exe") $DistClient -Force
Copy-Item (Join-Path $Root "client\README.txt") $DistClient -Force

Copy-Item (Join-Path $Root "server\target\release\backup-server.exe") $DistServer -Force
Copy-Item (Join-Path $Root "server\README.txt") $DistServer -Force

Copy-Item (Join-Path $Root "decrypt\target\release\backup-decrypt.exe") $DistDecrypt -Force
Copy-Item (Join-Path $Root "decrypt\decrypt.toml.example") $DistDecrypt -Force
Copy-Item (Join-Path $Root "decrypt\README.txt") $DistDecrypt -Force

Write-Host ""
Write-Host "Done. Binaries:" -ForegroundColor Green
Get-ChildItem -Recurse (Join-Path $Root "dist\win64") -File | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    Write-Host ("  {0,-40} sha256:{1}" -f $_.FullName.Replace($Root + "\", ""), $hash.Substring(0, 16) + "...")
}
Write-Host ""
Write-Host "Pack for GitHub Release: zip each subfolder under dist\win64\" -ForegroundColor Yellow
