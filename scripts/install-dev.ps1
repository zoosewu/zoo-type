#Requires -RunAsAdministrator
<#
.SYNOPSIS
  開發用安裝：把 release 建置結果放進 Program Files\ZooType\dev-<日期時間>-<commit>，再執行共用的 install.ps1。
.DESCRIPTION
  先以一般權限執行：cargo build --release --workspace、scripts/fetch-dict.sh
  再以系統管理員身分執行本腳本。

  每次都裝到新的資料夾：已開著的 App 繼續用舊版，重開 App 後才換新版（新舊版本並存）。
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root 'target\release'
$dict = Join-Path $root 'assets\dict'
$base = Join-Path $env:ProgramFiles 'ZooType'

$binaries = @("$release\zoo_type.dll", "$release\zoo-type-service.exe", "$release\zoo-type-settings.exe")
foreach ($f in $binaries + "$dict\static_dict.bin") {
    if (-not (Test-Path $f)) { throw "找不到 $f；請先 cargo build --release 並執行 scripts/fetch-dict.sh" }
}
# 本腳本不建置：最後一次建置比原始碼舊就拒絕安裝（cargo 只重建有變動的，所以看最新的那個檔），免得資料夾標著新的 commit、裝的卻是舊程式
$built = ($binaries | ForEach-Object { (Get-Item $_).LastWriteTime } | Measure-Object -Maximum).Maximum
$newest = Get-ChildItem (Join-Path $root 'crates'), (Join-Path $root 'Cargo.lock') -Recurse -File -Include *.rs, *.slint, *.toml, *.lock, *.txt, *.rc, *.ico |
    Where-Object { $_.FullName -notmatch '\\(target|tests|examples|benches)\\' } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($newest.LastWriteTime -gt $built) {
    throw "建置結果比原始碼舊（$($newest.FullName) 改過）；請先 cargo build --release --workspace"
}

$commit = (& git -C $root rev-parse --short HEAD 2>$null)
if (-not $commit) { $commit = 'unknown' }
if (& git -C $root status --porcelain 2>$null) { $commit = "$commit-dirty" }
$dest = Join-Path $base "dev-$(Get-Date -Format yyyyMMdd-HHmmss)-$commit"

Write-Host "安裝到 $dest"
Get-Process zoo-type-settings, zct-settings -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force -Path $dest, "$dest\dict" | Out-Null
Copy-Item "$release\zoo_type.dll", "$release\zoo-type-service.exe", "$release\zoo-type-settings.exe" $dest
Copy-Item "$dict\*" "$dest\dict"

& (Join-Path $PSScriptRoot 'install.ps1') -Dest $dest -Part All
Write-Host '已開啟的 App 繼續使用舊版，重新開啟後才會使用新版。'
