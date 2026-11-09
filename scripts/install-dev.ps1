#Requires -RunAsAdministrator
<#
.SYNOPSIS
  開發用安裝：把 release 建置結果安裝到 Program Files\ZCT\<版本資料夾> 並註冊 TSF 文字服務。
.DESCRIPTION
  先以一般權限執行：cargo build --release --workspace
  再以系統管理員身分執行本腳本。

  新舊版本並存（見 crates/zct-service/src/version.rs）：
  - 每次安裝到新的資料夾 dev-<日期時間>-<commit>，根目錄的 current.txt 記錄目前版本
  - 已開著的 App 繼續用舊版，重開 App 後才換新版；不會強制結束舊的服務程序
  - 舊版資料夾：沒有 App 使用就刪除；還在使用的整個保留，排到重開機時刪除
#>
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root 'target\release'
$dict = Join-Path $root 'assets\dict'
$base = Join-Path $env:ProgramFiles 'ZCT'
$tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}'

foreach ($f in @("$release\zct_tsf.dll", "$release\zct-service.exe", "$release\zct-settings.exe", "$dict\static_dict.bin")) {
    if (-not (Test-Path $f)) { throw "找不到 $f；請先 cargo build --release 並執行 scripts/fetch-dict.sh" }
}

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

# 刪除舊版：整個資料夾為單位；有任何程式使用中就整個排到重開機時刪除
function Remove-OldVersion([string]$path) {
    $files = @(Get-ChildItem $path -Recurse -File -Force -ErrorAction SilentlyContinue)
    $busy = @($files | Where-Object { $_.Extension -in '.dll', '.exe' } | Where-Object { Test-InUse $_.FullName })
    if ($busy.Count -eq 0) {
        Remove-Item $path -Recurse -Force
        Write-Host "  已刪除：$path"
        return
    }
    foreach ($f in $files) { Remove-AtReboot $f.FullName }
    Get-ChildItem $path -Recurse -Directory -Force | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object {
        Remove-AtReboot $_.FullName
    }
    Remove-AtReboot $path
    Write-Host "  使用中，重開機後刪除：$path"
}

# 版本資料夾：dev-<日期時間>-<commit>（每次都不同，不必覆蓋被載入中的檔案）
$commit = (& git -C $root rev-parse --short HEAD 2>$null)
if (-not $commit) { $commit = 'unknown' }
if (& git -C $root status --porcelain 2>$null) { $commit = "$commit-dirty" }
$name = "dev-$(Get-Date -Format yyyyMMdd-HHmmss)-$commit"
$dest = Join-Path $base $name
$dll = Join-Path $dest 'zct_tsf.dll'

Write-Host "安裝到 $dest"
Get-Process zct-settings -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force -Path $dest, "$dest\dict" | Out-Null
Copy-Item "$release\zct_tsf.dll", "$release\zct-service.exe", "$release\zct-settings.exe" $dest
Copy-Item "$dict\*" "$dest\dict"

# 協定版本（寫進 current.txt，讓舊版服務程序判斷要讓位還是改為唯讀）
$out = Join-Path $env:TEMP "zct-protocol-$PID.txt"
Start-Process -FilePath "$dest\zct-service.exe" -ArgumentList '--protocol' -RedirectStandardOutput $out -Wait -NoNewWindow
$protocol = (Get-Content $out -Raw).Trim()
Remove-Item $out -Force
if ($protocol -notmatch '^\d+$') { throw "讀不到協定版本：$protocol" }

Write-Host '註冊 TSF 文字服務'
$p = Start-Process regsvr32.exe -ArgumentList '/s', "`"$dll`"" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "regsvr32 失敗（$($p.ExitCode)）" }

Write-Host "目前版本：$name（協定 $protocol）"
Set-Content -Path (Join-Path $base 'current.txt') -Value @($name, $protocol) -Encoding ASCII

Write-Host '建立開始功能表捷徑'
$startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'ZCT 設定.lnk'
$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut($startMenu)
$lnk.TargetPath = "$dest\zct-settings.exe"
$lnk.WorkingDirectory = $dest
$lnk.Save()

Write-Host '設定登入時自動啟動服務程序'
Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -Value "`"$dest\zct-service.exe`""

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

Write-Host '清除舊版'
foreach ($item in Get-ChildItem $base -Force) {
    if ($item.Name -eq $name -or $item.Name -eq 'current.txt') { continue }
    if ($item.PSIsContainer -and (Test-Path (Join-Path $item.FullName 'zct_tsf.dll'))) {
        Remove-OldVersion $item.FullName
    }
}
# 舊的安裝方式：檔案直接放在根目錄（版本資料夾之前）
$legacy = @(Get-ChildItem $base -Force | Where-Object {
    $_.Name -ne $name -and $_.Name -ne 'current.txt' -and -not ($_.PSIsContainer -and (Test-Path (Join-Path $_.FullName 'zct_tsf.dll')))
})
if ($legacy.Count -gt 0) {
    $busy = @($legacy | Where-Object { -not $_.PSIsContainer -and $_.Extension -in '.dll', '.exe', '.old' } | Where-Object { Test-InUse $_.FullName })
    foreach ($item in $legacy) {
        if ($busy.Count -eq 0) {
            Remove-Item $item.FullName -Recurse -Force
        } else {
            $paths = @(Get-ChildItem $item.FullName -Recurse -Force -ErrorAction SilentlyContinue | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object FullName) + $item.FullName
            foreach ($p in $paths) { Remove-AtReboot $p }
        }
    }
    if ($busy.Count -eq 0) { Write-Host '  已刪除舊的安裝方式留下的檔案' } else { Write-Host '  舊的安裝方式留下的檔案使用中，重開機後刪除' }
}

Write-Host "完成：$dest"
Write-Host '已開啟的 App 繼續使用舊版，重新開啟後才會使用新版。'
