#Requires -RunAsAdministrator
<#
.SYNOPSIS
  解除安裝開發版：執行共用的 uninstall.ps1，並刪除空的安裝根目錄。正式版不受影響。
  開發版的資料（%APPDATA%\ZooType-dev 或自訂的資料根目錄）不會被刪除。
#>
$ErrorActionPreference = 'Continue'

$base = Join-Path $env:ProgramFiles 'ZooType-dev'
& (Join-Path $PSScriptRoot 'uninstall.ps1') -Channel dev -Base $base
if ((Test-Path $base) -and -not (Get-ChildItem $base -Force)) {
    Remove-Item $base -Force
}
