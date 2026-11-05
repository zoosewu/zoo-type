#Requires -RunAsAdministrator
<#
.SYNOPSIS
  開發用安裝：把 release 建置結果安裝到 Program Files\ZCT 並註冊 TSF 文字服務。
.DESCRIPTION
  先以一般權限執行：cargo build --release -p zct-tsf -p zct-service
  再以系統管理員身分執行本腳本。

  - DLL 被 App 載入時無法覆寫，但可以改名：舊檔改名成 *.old，下次安裝時清掉
  - 服務程序由 DLL 在第一次按鍵時自動啟動（只從一般權限的桌面程式啟動）
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root 'target\release'
$dict = Join-Path $root 'assets\dict'
$dest = Join-Path $env:ProgramFiles 'ZCT'
$dll = Join-Path $dest 'zct_tsf.dll'
$tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}'

foreach ($f in @("$release\zct_tsf.dll", "$release\zct-service.exe", "$dict\static_dict.bin")) {
    if (-not (Test-Path $f)) { throw "找不到 $f；請先 cargo build --release 並執行 scripts/fetch-dict.sh" }
}

Write-Host '停止服務程序'
Get-Process zct-service -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

New-Item -ItemType Directory -Force -Path $dest, "$dest\dict" | Out-Null

# 清掉之前改名、現在已沒有 App 載入的舊 DLL
Get-ChildItem $dest -Filter '*.old' -ErrorAction SilentlyContinue | ForEach-Object {
    try { Remove-Item $_.FullName -Force } catch { }
}
if (Test-Path $dll) {
    $old = "$dll.$(Get-Date -Format yyyyMMddHHmmss).old"
    Move-Item $dll $old
}

Copy-Item "$release\zct_tsf.dll" $dest
Copy-Item "$release\zct-service.exe" $dest -Force
Copy-Item "$dict\*" "$dest\dict" -Force

Write-Host '註冊 TSF 文字服務'
$p = Start-Process regsvr32.exe -ArgumentList '/s', "`"$dll`"" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "regsvr32 失敗（$($p.ExitCode)）" }

Write-Host '設定登入時自動啟動服務程序'
Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -Value "`"$dest\zct-service.exe`""

Write-Host '加入中文（台灣）鍵盤清單'
$list = Get-WinUserLanguageList
$zh = $list | Where-Object LanguageTag -eq 'zh-Hant-TW'
if (-not $zh) {
    $list.Add('zh-Hant-TW')
    $zh = $list | Where-Object LanguageTag -eq 'zh-Hant-TW'
}
if (-not ($zh.InputMethodTips -contains $tip)) {
    $zh.InputMethodTips.Add($tip)
    Set-WinUserLanguageList $list -Force
}

Write-Host "完成：$dest"
Write-Host '用 Win + 空白鍵 切換到「ZCT 注音」。已開啟的 App 需要重新開啟才會載入新版 DLL。'
