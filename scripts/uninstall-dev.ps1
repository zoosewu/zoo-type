#Requires -RunAsAdministrator
<#
.SYNOPSIS
  解除安裝開發版：從鍵盤清單移除、解除 TSF 註冊、停止服務程序、刪除所有版本的檔案。
  使用者資料（%APPDATA%\zct 或自訂的資料根目錄）不會被刪除。
  仍被 App 載入的檔案排到重開機時刪除。
#>
$ErrorActionPreference = 'Continue'

$base = Join-Path $env:ProgramFiles 'ZCT'
$tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}'

Add-Type -Namespace Zct -Name Native -MemberDefinition @'
[DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
public static extern bool MoveFileEx(string existing, string replacement, int flags);
'@

# 排到重開機時刪除（被 App 載入中的檔案無法立即刪除）
# 注意：PowerShell 把 $null 傳給 string 參數時會變成空字串，MoveFileEx 會因此失敗，必須用 [NullString]::Value
function Remove-AtReboot([string]$path) {
    if (-not [Zct.Native]::MoveFileEx($path, [NullString]::Value, 4)) {
        Write-Warning "無法排程刪除 $path（錯誤 $([Runtime.InteropServices.Marshal]::GetLastWin32Error())）"
    }
}

Write-Host '從鍵盤清單移除'
$list = Get-WinUserLanguageList
foreach ($lang in $list) {
    if ($lang.InputMethodTips -contains $tip) { $null = $lang.InputMethodTips.Remove($tip) }
}
Set-WinUserLanguageList $list -Force

# 目前註冊的 DLL：current.txt 的版本資料夾，或舊的安裝方式放在根目錄的
$dlls = @()
$current = Join-Path $base 'current.txt'
if (Test-Path $current) { $dlls += Join-Path (Join-Path $base ((Get-Content $current)[0])) 'zct_tsf.dll' }
$dlls += Join-Path $base 'zct_tsf.dll'
foreach ($dll in $dlls | Where-Object { Test-Path $_ }) {
    Write-Host "解除 TSF 註冊：$dll"
    Start-Process regsvr32.exe -ArgumentList '/u', '/s', "`"$dll`"" -Wait
}

Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -ErrorAction SilentlyContinue
Get-Process zct-service, zct-settings -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'ZCT 設定.lnk') -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 300

try {
    Remove-Item $base -Recurse -Force -ErrorAction Stop
    Write-Host "已刪除：$base"
} catch {
    $paths = @(Get-ChildItem $base -Recurse -Force -ErrorAction SilentlyContinue | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object FullName) + $base
    foreach ($p in $paths) { Remove-AtReboot $p }
    Write-Host "部分檔案仍被 App 使用，重開機後刪除：$base"
}
Write-Host '完成'
