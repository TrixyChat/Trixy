$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RootDir

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust/Cargo is required. Install Rust from https://rustup.rs and run this script again."
}

cargo packager --version *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Host "Installing cargo-packager..."
    cargo install cargo-packager --version 0.11.8 --locked
}

if (Test-Path dist) {
    Remove-Item -Recurse -Force dist
}

cargo packager --release --formats nsis
Write-Host ""
Write-Host "Installer build complete. Look in: $RootDir\dist"
