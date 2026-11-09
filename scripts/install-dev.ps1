#Requires -RunAsAdministrator
<#
.SYNOPSIS
  開發用安裝：把 release 建置結果放進 Program Files\ZCT\dev-<日期時間>-<commit>，再執行共用的 install.ps1。
.DESCRIPTION
  先以一般權限執行：cargo build --release --workspace、scripts/fetch-dict.sh
  再以系統管理員身分執行本腳本。

  每次都裝到新的資料夾：已開著的 App 繼續用舊版，重開 App 後才換新版（新舊版本並存）。
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root 'target\release'
$dict = Join-Path $root 'assets\dict'
$base = Join-Path $env:ProgramFiles 'ZCT'

foreach ($f in @("$release\zct_tsf.dll", "$release\zct-service.exe", "$release\zct-settings.exe", "$dict\static_dict.bin")) {
    if (-not (Test-Path $f)) { throw "找不到 $f；請先 cargo build --release 並執行 scripts/fetch-dict.sh" }
}

$commit = (& git -C $root rev-parse --short HEAD 2>$null)
if (-not $commit) { $commit = 'unknown' }
if (& git -C $root status --porcelain 2>$null) { $commit = "$commit-dirty" }
$dest = Join-Path $base "dev-$(Get-Date -Format yyyyMMdd-HHmmss)-$commit"

Write-Host "安裝到 $dest"
Get-Process zct-settings -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force -Path $dest, "$dest\dict" | Out-Null
Copy-Item "$release\zct_tsf.dll", "$release\zct-service.exe", "$release\zct-settings.exe" $dest
Copy-Item "$dict\*" "$dest\dict"

& (Join-Path $PSScriptRoot 'install.ps1') -Dest $dest -Part All
Write-Host '已開啟的 App 繼續使用舊版，重新開啟後才會使用新版。'
