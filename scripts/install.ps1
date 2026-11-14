#Requires -Version 5.1
<#
.SYNOPSIS
  安裝核心（開發版 install-dev.ps1 與正式安裝程式共用）：版本資料夾裡的檔案已經放好，這裡負責其餘步驟。
.PARAMETER Dest
  版本資料夾，例如 C:\Program Files\ZooType\0.1.0；裡面要有 zoo_type.dll、zoo-type-service.exe、zoo-type-settings.exe、dict\
.PARAMETER Part
  System：需要系統管理員權限的部分（註冊 TSF、current.txt、清除舊版）
  User：每個使用者各自的設定（開機自動啟動、鍵盤清單、學習資料位置），要以使用者本人的身分執行
  All：兩者（開發版：系統管理員就是使用者本人）
.PARAMETER Channel
  official（正式版，預設）或 dev（開發版）；必須和版本資料夾裡的程式相同（zoo-type-service.exe --channel）
.PARAMETER DataRoot
  初次安裝時使用者選的學習資料位置（User 部分使用）。和預設的 %APPDATA%\ZooType 不同時寫進
  %APPDATA%\ZooType\location.toml；已有 location.toml 時一律不覆蓋（更新時沿用原本的位置）。
.DESCRIPTION
  新舊版本並存（見 crates/zct-service/src/version.rs）：
  - 根目錄的 current.txt 記錄目前版本資料夾與協定版本
  - 已開著的 App 繼續用舊版；舊版資料夾沒有程式使用就刪除，使用中的整個排到重開機時刪除
#>
param(
    [Parameter(Mandatory)] [string]$Dest,
    [ValidateSet('All', 'System', 'User')] [string]$Part = 'All',
    [ValidateSet('official', 'dev')] [string]$Channel = 'official',
    [string]$DataRoot
)
$ErrorActionPreference = 'Stop'

# 正式版與開發版是兩套獨立的輸入法（crates/zct-core/src/channel.rs），各自的識別：
# 安裝 / 資料資料夾名稱（也是登入自動啟動的名稱）、鍵盤清單中的 TIP（語言:{CLSID}{語言設定檔}）
$channels = @{
    official = @{ Dir = 'ZooType'; Tip = '0404:{68E048CC-2635-4A97-8A2F-1599AF3AFCF5}{D890A24E-551E-4D5F-A0F1-745BE13B6287}' }
    dev      = @{ Dir = 'ZooType-dev'; Tip = '0404:{A45461CC-EC30-4B88-BADA-0F20A6FD0C19}{B28977F5-A5AC-4C38-8817-FC51E1F79919}' }
}
$ch = $channels[$Channel]

$Dest = (Resolve-Path $Dest).Path
$base = Split-Path -Parent $Dest
$name = Split-Path -Leaf $Dest
$dll = Join-Path $Dest 'zoo_type.dll'
$tip = $ch.Tip
$official = $Channel -eq 'official'
# 版本資料夾之前的安裝方式直接放在根目錄的檔案
$legacyNames = @('zct_tsf.dll', 'zct-service.exe', 'zct-settings.exe', 'zct.ico', 'dict')
# 改名前的安裝根目錄
$legacyBase = Join-Path $env:ProgramFiles 'ZCT'

# 版本資料夾：裡面有輸入法 DLL（改名前是 zct_tsf.dll）
function Test-VersionDir([System.IO.FileSystemInfo]$item) {
    $item.PSIsContainer -and ((Test-Path (Join-Path $item.FullName 'zoo_type.dll')) -or (Test-Path (Join-Path $item.FullName 'zct_tsf.dll')))
}

foreach ($f in @('zoo_type.dll', 'zoo-type-service.exe', 'zoo-type-settings.exe', 'dict\static_dict.bin')) {
    if (-not (Test-Path (Join-Path $Dest $f))) { throw "版本資料夾缺少 $f：$Dest" }
}

# 程式說的版本要和要裝的一致：開發版的程式裝成正式版（或反過來）會蓋掉另一套的註冊
function Get-ServiceOutput([string]$arg) {
    $out = Join-Path $env:TEMP "zct-$arg-$PID.txt"
    Start-Process -FilePath (Join-Path $Dest 'zoo-type-service.exe') -ArgumentList "--$arg" -RedirectStandardOutput $out -Wait -NoNewWindow
    $value = (Get-Content $out -Raw).Trim()
    Remove-Item $out -Force
    $value
}
$built = Get-ServiceOutput 'channel'
if ($built -ne $Channel) { throw "版本資料夾裡是 $built 的程式，不能裝成 $Channel：$Dest" }

# 自動更新（只有正式版）：以 SYSTEM 執行 zoo-type-updater 的排程工作，一般使用者可以啟動它
# （安全性靠 updater 驗證安裝檔的簽章與版號，見 crates/zct-update）。每次安裝都指向新版本的 updater
function Register-Updater {
    $updater = Join-Path $Dest 'zoo-type-updater.exe'
    if (-not (Test-Path $updater)) { return }
    Write-Host '建立自動更新的排程工作'
    $svc = New-Object -ComObject Schedule.Service
    $svc.Connect()
    $def = $svc.NewTask(0)
    $def.RegistrationInfo.Description = '動物園輸入法：安裝已簽章的更新'
    $def.Settings.AllowDemandStart = $true
    $def.Settings.DisallowStartIfOnBatteries = $false
    $def.Settings.StopIfGoingOnBatteries = $false
    $def.Settings.ExecutionTimeLimit = 'PT30M'
    $def.Settings.MultipleInstances = 2   # 執行中時忽略新的啟動
    $def.Principal.RunLevel = 1           # 最高權限
    $action = $def.Actions.Create(0)
    $action.Path = $updater
    # SYSTEM 與系統管理員完全控制；Authenticated Users 可以讀取與執行（啟動）這個工作
    $sddl = 'D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;AU)'
    $null = $svc.GetFolder('\').RegisterTaskDefinition('ZooType Updater', $def, 6, 'SYSTEM', $null, 5, $sddl)
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
    $protocol = Get-ServiceOutput 'protocol'
    if ($protocol -notmatch '^\d+$') { throw "讀不到協定版本：$protocol" }

    # 圖示用固定路徑的複本（見 crates/zct-tsf/src/register.rs 的 icon_path）；被讀取中而無法覆蓋時沿用舊的（圖示一樣）
    try { Copy-Item $dll (Join-Path $base 'zoo-type-icon.dll') -Force -ErrorAction Stop } catch { Write-Host '  沿用原本的圖示檔' }

    Write-Host '註冊 TSF 文字服務'
    $p = Start-Process regsvr32.exe -ArgumentList '/s', "`"$dll`"" -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "regsvr32 失敗（$($p.ExitCode)）" }

    Write-Host "目前版本：$name（協定 $protocol）"

    Set-Content -Path (Join-Path $base 'current.txt') -Value @($name, $protocol) -Encoding ASCII

    if ($official) { Register-Updater }

    Write-Host '清除舊版'
    foreach ($item in Get-ChildItem $base -Directory -Force) {
        if ($item.Name -ne $name -and (Test-VersionDir $item)) {
            Remove-Old @($item) $item.FullName
        }
    }
    if (-not $official) { return }
    $legacy = @(Get-ChildItem $base -Force | Where-Object { $_.Name -in $legacyNames -or $_.Name -like 'zct_tsf.dll.*.old' })
    if ($legacy.Count -gt 0) { Remove-Old $legacy '舊的安裝方式留下的檔案' }
    # 改名前的安裝根目錄（C:\Program Files\ZCT）整個移除
    if ((Test-Path $legacyBase) -and ($legacyBase -ne $base)) {
        $items = @(Get-ChildItem $legacyBase -Force)
        if ($items.Count -gt 0) { Remove-Old $items $legacyBase }
        if (Test-Path $legacyBase) {
            if (@(Get-ChildItem $legacyBase -Force).Count -eq 0) { Remove-Item $legacyBase -Force } else { Remove-AtReboot $legacyBase }
        }
    }
}

# 資料位置：寫進 %APPDATA%\<資料夾>\location.toml（zct-core 的 paths.rs 依此尋找資料根目錄）
# 只在初次安裝（沒有舊資料）時呼叫：不能在服務程序搬移舊的 %APPDATA%\zct 之前先建立新資料夾
function Set-DataRoot([string]$path) {
    $pointerDir = Join-Path $env:APPDATA $ch.Dir
    $pointer = Join-Path $pointerDir 'location.toml'
    if (-not $path) { return }
    if (Test-Path $pointer) {
        Write-Host "沿用原本的學習資料位置（$pointer）"
        return
    }
    $full = [IO.Path]::GetFullPath($path).TrimEnd('\')
    if ($full -ieq $pointerDir) { return }
    New-Item -ItemType Directory -Force -Path $pointerDir, $full | Out-Null
    # TOML 基本字串：反斜線與雙引號要跳脫
    $escaped = $full.Replace('\', '\\').Replace('"', '\"')
    # 不加 BOM（PowerShell 5.1 的 -Encoding UTF8 會加；zct-core 雖然也能讀，仍以標準格式為準）
    [IO.File]::WriteAllText($pointer, "data_root = `"$escaped`"`r`n", (New-Object Text.UTF8Encoding $false))
    Write-Host "學習資料位置：$full"
}

function Install-User {
    Set-DataRoot $DataRoot
    if ($official) {
        # 不建立開始功能表捷徑：設定程式從工作列輸入法圖示的右鍵選單開啟；清掉舊版建立的捷徑
        foreach ($n in '動物園輸入法 設定.lnk', 'ZCT 設定.lnk') {
            Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) $n) -ErrorAction SilentlyContinue
        }
        Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'ZCT' -ErrorAction SilentlyContinue
    }

    Write-Host '設定登入時自動啟動服務程序'
    Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name $ch.Dir -Value "`"$(Join-Path $Dest 'zoo-type-service.exe')`""

    # 正式版排第一位（切到中文時就是它）；開發版加在最後，不搶正式版的位置
    Write-Host '加入中文（台灣）鍵盤清單'
    $list = Get-WinUserLanguageList
    $zh = $list | Where-Object LanguageTag -eq 'zh-Hant-TW'
    if (-not $zh) {
        $list.Add('zh-Hant-TW')
        $zh = $list | Where-Object LanguageTag -eq 'zh-Hant-TW'
    }
    if ($official -and $zh.InputMethodTips[0] -ne $tip) {
        $null = $zh.InputMethodTips.Remove($tip)
        $zh.InputMethodTips.Insert(0, $tip)
        Set-WinUserLanguageList $list -Force
    } elseif (-not $official -and $zh.InputMethodTips -notcontains $tip) {
        $zh.InputMethodTips.Add($tip)
        Set-WinUserLanguageList $list -Force
    }
}

if ($Part -in 'All', 'System') { Install-System }
if ($Part -in 'All', 'User') { Install-User }
Write-Host "完成：$Dest"
