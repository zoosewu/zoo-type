#Requires -Version 5.1
<#
.SYNOPSIS
  安裝核心（開發版 install-dev.ps1 與正式安裝程式共用）：版本資料夾裡的檔案已經放好，這裡負責其餘步驟。
.PARAMETER Dest
  版本資料夾，例如 C:\Program Files\ZCT\0.1.0；裡面要有 zct_tsf.dll、zct-service.exe、zct-settings.exe、dict\
.PARAMETER Part
  System：需要系統管理員權限的部分（註冊 TSF、current.txt、清除舊版）
  User：每個使用者各自的設定（開機自動啟動、開始功能表、鍵盤清單），要以使用者本人的身分執行
  All：兩者（開發版：系統管理員就是使用者本人）
.DESCRIPTION
  新舊版本並存（見 crates/zct-service/src/version.rs）：
  - 根目錄的 current.txt 記錄目前版本資料夾與協定版本
  - 已開著的 App 繼續用舊版；舊版資料夾沒有程式使用就刪除，使用中的整個排到重開機時刪除
#>
param(
    [Parameter(Mandatory)] [string]$Dest,
    [ValidateSet('All', 'System', 'User')] [string]$Part = 'All'
)
$ErrorActionPreference = 'Stop'

$Dest = (Resolve-Path $Dest).Path
$base = Split-Path -Parent $Dest
$name = Split-Path -Leaf $Dest
$dll = Join-Path $Dest 'zct_tsf.dll'
$tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}'
# 版本資料夾之前的安裝方式直接放在根目錄的檔案
$legacyNames = @('zct_tsf.dll', 'zct-service.exe', 'zct-settings.exe', 'zct.ico', 'dict')

foreach ($f in @('zct_tsf.dll', 'zct-service.exe', 'zct-settings.exe', 'dict\static_dict.bin')) {
    if (-not (Test-Path (Join-Path $Dest $f))) { throw "版本資料夾缺少 $f：$Dest" }
}

function Install-System {
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

    # 檔案是否被 App 載入或正在執行（無法以獨占方式開啟）
    function Test-InUse([string]$path) {
        try { [IO.File]::Open($path, 'Open', 'ReadWrite', 'None').Close(); return $false } catch { return $true }
    }

    # 刪除舊版（一組檔案為單位）：有任何程式使用中就整組排到重開機時刪除，
    # 避免只刪掉一部分（例如服務程序被刪了，還在用舊 DLL 的 App 就叫不起服務程序）
    function Remove-Old([System.IO.FileSystemInfo[]]$items, [string]$label) {
        $files = @($items | ForEach-Object {
            if ($_.PSIsContainer) { Get-ChildItem $_.FullName -Recurse -File -Force -ErrorAction SilentlyContinue } else { $_ }
        })
        $busy = @($files | Where-Object { $_.Extension -in '.dll', '.exe', '.old' } | Where-Object { Test-InUse $_.FullName })
        if ($busy.Count -eq 0) {
            $items | ForEach-Object { Remove-Item $_.FullName -Recurse -Force }
            Write-Host "  已刪除：$label"
            return
        }
        foreach ($f in $files) { Remove-AtReboot $f.FullName }
        $items | Where-Object PSIsContainer | ForEach-Object {
            Get-ChildItem $_.FullName -Recurse -Directory -Force | Sort-Object { $_.FullName.Length } -Descending |
                ForEach-Object { Remove-AtReboot $_.FullName }
            Remove-AtReboot $_.FullName
        }
        Write-Host "  使用中，重開機後刪除：$label"
    }

    # 協定版本（寫進 current.txt，讓舊版服務程序判斷要讓位還是改為唯讀）
    $out = Join-Path $env:TEMP "zct-protocol-$PID.txt"
    Start-Process -FilePath (Join-Path $Dest 'zct-service.exe') -ArgumentList '--protocol' -RedirectStandardOutput $out -Wait -NoNewWindow
    $protocol = (Get-Content $out -Raw).Trim()
    Remove-Item $out -Force
    if ($protocol -notmatch '^\d+$') { throw "讀不到協定版本：$protocol" }

    Write-Host '註冊 TSF 文字服務'
    $p = Start-Process regsvr32.exe -ArgumentList '/s', "`"$dll`"" -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "regsvr32 失敗（$($p.ExitCode)）" }

    Write-Host "目前版本：$name（協定 $protocol）"
    Set-Content -Path (Join-Path $base 'current.txt') -Value @($name, $protocol) -Encoding ASCII

    Write-Host '清除舊版'
    foreach ($item in Get-ChildItem $base -Directory -Force) {
        if ($item.Name -ne $name -and (Test-Path (Join-Path $item.FullName 'zct_tsf.dll'))) {
            Remove-Old @($item) $item.FullName
        }
    }
    $legacy = @(Get-ChildItem $base -Force | Where-Object { $_.Name -in $legacyNames -or $_.Name -like 'zct_tsf.dll.*.old' })
    if ($legacy.Count -gt 0) { Remove-Old $legacy '舊的安裝方式留下的檔案' }
}

function Install-User {
    Write-Host '建立開始功能表捷徑'
    $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) '動物園輸入法 設定.lnk'
    # 舊名稱的捷徑
    Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'ZCT 設定.lnk') -ErrorAction SilentlyContinue
    $shell = New-Object -ComObject WScript.Shell
    $lnk = $shell.CreateShortcut($startMenu)
    $lnk.TargetPath = Join-Path $Dest 'zct-settings.exe'
    $lnk.WorkingDirectory = $Dest
    $lnk.Save()

    Write-Host '設定登入時自動啟動服務程序'
    Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -Value "`"$(Join-Path $Dest 'zct-service.exe')`""

    Write-Host '加入中文（台灣）鍵盤清單，並排在第一位（Alt+Shift 切到中文時就是「動物園輸入法」）'
    $list = Get-WinUserLanguageList
    $zh = $list | Where-Object LanguageTag -eq 'zh-Hant-TW'
    if (-not $zh) {
        $list.Add('zh-Hant-TW')
        $zh = $list | Where-Object LanguageTag -eq 'zh-Hant-TW'
    }
    if ($zh.InputMethodTips[0] -ne $tip) {
        $null = $zh.InputMethodTips.Remove($tip)
        $zh.InputMethodTips.Insert(0, $tip)
        Set-WinUserLanguageList $list -Force
    }
}

if ($Part -in 'All', 'System') { Install-System }
if ($Part -in 'All', 'User') { Install-User }
Write-Host "完成：$Dest"
