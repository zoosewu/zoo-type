#Requires -RunAsAdministrator
<#
.SYNOPSIS
  解除安裝開發版：從鍵盤清單移除、解除 TSF 註冊、停止服務程序、刪除檔案。
  使用者資料（%APPDATA%\zct 或自訂的資料根目錄）不會被刪除。
#>
$ErrorActionPreference = 'Continue'

$dest = Join-Path $env:ProgramFiles 'ZCT'
$dll = Join-Path $dest 'zct_tsf.dll'
$tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}'

Write-Host '從鍵盤清單移除'
$list = Get-WinUserLanguageList
foreach ($lang in $list) {
    if ($lang.InputMethodTips -contains $tip) { $null = $lang.InputMethodTips.Remove($tip) }
}
Set-WinUserLanguageList $list -Force

if (Test-Path $dll) {
    Write-Host '解除 TSF 註冊'
    Start-Process regsvr32.exe -ArgumentList '/u', '/s', "`"$dll`"" -Wait
}

Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -ErrorAction SilentlyContinue
Get-Process zct-service -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

# 仍被 App 載入的 DLL 無法刪除：改名，登出或重開機後再手動刪除
Get-ChildItem $dest -Recurse -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | ForEach-Object {
    try { Remove-Item $_.FullName -Force -Recurse }
    catch { if ($_.TargetObject -notlike '*.old') { Move-Item $_.TargetObject "$($_.TargetObject).old" -ErrorAction SilentlyContinue } }
}
try { Remove-Item $dest -Force -Recurse } catch { Write-Host "部分檔案仍被使用中，登出後可刪除：$dest" }
Write-Host '完成'
