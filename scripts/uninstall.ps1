#Requires -Version 5.1
<#
.SYNOPSIS
  解除安裝核心（開發版 uninstall-dev.ps1 與正式安裝程式共用）。
.DESCRIPTION
  從鍵盤清單移除、解除 TSF 註冊、停止服務程序、刪除所有版本資料夾與 current.txt。
  使用者資料（%APPDATA%\ZooType 或自訂的資料根目錄）不會被刪除。
  仍被 App 載入的檔案排到重開機時刪除。根目錄本身與其他檔案（例如安裝程式的解除安裝程式）不動。
.PARAMETER Channel
  official（正式版，預設）或 dev（開發版）：只移除這一套，另一套不受影響
.PARAMETER Base
  安裝根目錄，預設 C:\Program Files\ZooType（開發版 ZooType-dev）；正式版會一併清除改名前的 C:\Program Files\ZCT
#>
param(
    [ValidateSet('official', 'dev')] [string]$Channel = 'official',
    [string]$Base
)
$ErrorActionPreference = 'Continue'

# 正式版與開發版是兩套獨立的輸入法（crates/zct-core/src/channel.rs），各自的識別：
# 安裝 / 資料資料夾名稱（也是登入自動啟動的名稱）、鍵盤清單中的 TIP（語言:{CLSID}{語言設定檔}）
$channels = @{
    official = @{ Dir = 'ZooType'; Tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}' }
    dev      = @{ Dir = 'ZooType-dev'; Tip = '0404:{A45461CC-EC30-4B88-BADA-0F20A6FD0C19}{B28977F5-A5AC-4C38-8817-FC51E1F79919}' }
}
$ch = $channels[$Channel]
if (-not $Base) { $Base = Join-Path $env:ProgramFiles $ch.Dir }
$official = $Channel -eq 'official'
$tip = $ch.Tip
$legacyNames = @('zct_tsf.dll', 'zct-service.exe', 'zct-settings.exe', 'zct.ico', 'dict')

Add-Type -Namespace Zct -Name Native -MemberDefinition @'
[DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
public static extern bool MoveFileEx(string existing, string replacement, int flags);
'@

# 排到重開機時刪除；PowerShell 的 $null 傳給 string 參數會變成空字串，必須用 [NullString]::Value
function Remove-AtReboot([string]$path) {
    if (-not [Zct.Native]::MoveFileEx($path, [NullString]::Value, 4)) {
        Write-Warning "無法排程刪除 $path（錯誤 $([Runtime.InteropServices.Marshal]::GetLastWin32Error())）"
    }
}

function Remove-OrSchedule([string]$path) {
    try {
        Remove-Item $path -Recurse -Force -ErrorAction Stop
    } catch {
        $paths = @(Get-ChildItem $path -Recurse -Force -ErrorAction SilentlyContinue |
            Sort-Object { $_.FullName.Length } -Descending | ForEach-Object FullName) + $path
        foreach ($p in $paths) { if (Test-Path $p) { Remove-AtReboot $p } }
        Write-Host "  使用中，重開機後刪除：$path"
    }
}

Write-Host '從鍵盤清單移除'
$list = Get-WinUserLanguageList
foreach ($lang in $list) {
    if ($lang.InputMethodTips -contains $tip) { $null = $lang.InputMethodTips.Remove($tip) }
}
Set-WinUserLanguageList $list -Force

# 目前註冊的 DLL：current.txt 的版本資料夾，或舊的安裝方式放在根目錄的
$legacyBase = Join-Path $env:ProgramFiles 'ZCT'
$bases = if ($official) { @($Base, $legacyBase) | Select-Object -Unique } else { @($Base) }
$dlls = @()
foreach ($b in $bases) {
    $current = Join-Path $b 'current.txt'
    if (Test-Path $current) {
        $dir = Join-Path $b ((Get-Content $current)[0])
        $dlls += (Join-Path $dir 'zoo_type.dll'), (Join-Path $dir 'zct_tsf.dll')
    }
    $dlls += Join-Path $b 'zct_tsf.dll'
}
foreach ($dll in $dlls | Where-Object { Test-Path $_ }) {
    Write-Host "解除 TSF 註冊：$dll"
    Start-Process regsvr32.exe -ArgumentList '/u', '/s', "`"$dll`"" -Wait
}

Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $ch.Dir -ErrorAction SilentlyContinue
if ($official) {
    Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -ErrorAction SilentlyContinue
    foreach ($n in '動物園輸入法 設定.lnk', 'ZCT 設定.lnk') {
        Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) $n) -ErrorAction SilentlyContinue
    }
}
# 只結束這一套的程式（看執行檔所在的資料夾）
Get-Process zoo-type-service, zoo-type-settings, zct-service, zct-settings -ErrorAction SilentlyContinue |
    Where-Object { $p = $_.Path; $p -and ($bases | Where-Object { $p.StartsWith("$_\", 'OrdinalIgnoreCase') }) } |
    Stop-Process -Force
Start-Sleep -Milliseconds 300

if ($official) {
    try {
        $svc = New-Object -ComObject Schedule.Service
        $svc.Connect()
        $svc.GetFolder('\').DeleteTask('ZooType Updater', 0)
        Write-Host '已移除自動更新的排程工作'
    } catch {}
}

Write-Host '刪除程式檔案'
# 改名前的安裝根目錄整個移除
if ($official -and (Test-Path $legacyBase) -and ($legacyBase -ne $Base)) { Remove-OrSchedule $legacyBase }
if (Test-Path $Base) {
    foreach ($item in Get-ChildItem $Base -Force) {
        $isVersion = $item.PSIsContainer -and ((Test-Path (Join-Path $item.FullName 'zoo_type.dll')) -or (Test-Path (Join-Path $item.FullName 'zct_tsf.dll')))
        if ($isVersion -or $item.Name -in $legacyNames -or $item.Name -like 'zct_tsf.dll.*.old' -or $item.Name -in 'current.txt', 'update', 'zoo-type-icon.dll') {
            Remove-OrSchedule $item.FullName
        }
    }
}
Write-Host '完成'
