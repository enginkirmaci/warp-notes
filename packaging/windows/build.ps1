#Requires -Version 5.1
# Builds Warp Notes for Windows and stages the runnable file in out\:
# warp-notes.exe. No zip, no installer - out\ is the deliverable;
# build.ps1 -Installer also runs the Inno Setup script.
# Run from a Windows machine - GPUI has no Linux->Windows cross build.
param(
    [switch]$Installer
)
$ErrorActionPreference = 'Stop'

$here   = $PSScriptRoot
$repo   = Split-Path -Parent (Split-Path -Parent $here)   # script lives in packaging\windows
$outDir = Join-Path $here 'out'

Push-Location $repo
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    # cargo may place artifacts off-repo (e.g. CARGO_TARGET_DIR on a fast
    # local drive while the repo lives on a network share)
    $targetDir = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).target_directory
} finally {
    Pop-Location
}

$exe = Join-Path $targetDir 'release\warp-notes.exe'
if (-not (Test-Path $exe)) { throw "release exe not found: $exe" }

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
Get-ChildItem $outDir -Force | Remove-Item -Recurse -Force

Copy-Item $exe (Join-Path $outDir 'warp-notes.exe')

if ($Installer) {
    $version = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages |
        Where-Object name -eq 'warp-notes' | Select-Object -ExpandProperty version
    iscc /DMyAppVersion=$version (Join-Path $here 'warp-notes.iss')
}

Write-Host "staged in $outDir"
