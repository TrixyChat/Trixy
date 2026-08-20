$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $RootDir
cargo build --release
Write-Host "Built: $RootDir\target\release\trixy.exe"
