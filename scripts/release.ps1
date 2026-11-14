#Requires -Version 5.1
<#
.SYNOPSIS
  發布新版：改 Cargo.toml 的版號、commit、打 v<版號> 標籤（不推送）。
.DESCRIPTION
  在乾淨的 main 上執行。之後 git push origin main v<版號>，GitHub Actions 的 release.yml 會建置、簽章並建立 Release。
  已安裝的正式版會在一天內查到新版，輸入法圖示的選單出現「更新到 v<版號>」。
.EXAMPLE
  scripts/release.ps1 0.1.1
#>
param([Parameter(Mandatory)] [string]$Version)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { throw "版號格式不對：$Version（例如 0.1.1、0.2.0-rc.1）" }
$branch = (git -C $root rev-parse --abbrev-ref HEAD).Trim()
if ($branch -ne 'main') { throw "請在 main 上發布（目前在 $branch）" }
if (git -C $root status --porcelain) { throw '有還沒 commit 的修改' }
if (git -C $root tag --list "v$Version") { throw "標籤 v$Version 已經存在" }

$cargo = Join-Path $root 'Cargo.toml'
$text = [IO.File]::ReadAllText($cargo)
$new = [regex]::Replace($text, '(?ms)(^\[workspace\.package\].*?^version\s*=\s*")[^"]+(")', "`${1}$Version`${2}", 1)
if ($new -eq $text) { throw 'Cargo.toml 的版號沒有改到（已經是這個版號？）' }
[IO.File]::WriteAllText($cargo, $new)
# 更新 Cargo.lock 裡 workspace 套件的版號
cargo update --workspace --manifest-path $cargo
if ($LASTEXITCODE -ne 0) { throw 'cargo update 失敗' }

git -C $root commit -q -am "版本 $Version"
git -C $root tag -a "v$Version" -m "動物園輸入法 $Version"
Write-Host "完成：v$Version。推送：git push origin main v$Version"
