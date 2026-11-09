#Requires -RunAsAdministrator
<#
.SYNOPSIS
  解除安裝開發版：執行共用的 uninstall.ps1，並刪除空的安裝根目錄。
  使用者資料（%APPDATA%\ZooType 或自訂的資料根目錄）不會被刪除。
#>
$ErrorActionPreference = 'Continue'

$base = Join-Path $env:ProgramFiles 'ZooType'
& (Join-Path $PSScriptRoot 'uninstall.ps1') -Base $base
if ((Test-Path $base) -and -not (Get-ChildItem $base -Force)) {
    Remove-Item $base -Force
}
