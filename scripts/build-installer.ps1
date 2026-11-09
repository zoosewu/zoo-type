#Requires -Version 5.1
<#
.SYNOPSIS
  產生正式安裝檔 target\installer\zoo-type-<版本>-windows-x64-setup.exe
.DESCRIPTION
  先執行：cargo build --release --workspace、scripts/fetch-dict.sh
  版號取自 Cargo.toml 的 [workspace.package] version。
  需要 Inno Setup 6（ISCC.exe）：本機 winget install JRSoftware.InnoSetup；CI 用 choco 或預裝的版本。
.PARAMETER Iscc
  ISCC.exe 路徑；不指定時自動尋找
#>
param([string]$Iscc)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$release = Join-Path $root 'target\release'
$dict = Join-Path $root 'assets\dict'
$stage = Join-Path $root 'target\installer\stage'

# 版號：[workspace.package] 的 version
$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($cargo -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { throw '讀不到 Cargo.toml 的 workspace 版號' }
$version = $Matches[1]

if (-not $Iscc) {
    $Iscc = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Iscc) { $Iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source }
}
if (-not $Iscc) { throw '找不到 ISCC.exe（Inno Setup 6）' }

foreach ($f in @("$release\zct_tsf.dll", "$release\zct-service.exe", "$release\zct-settings.exe", "$dict\static_dict.bin", "$dict\tsi.csv")) {
    if (-not (Test-Path $f)) { throw "找不到 $f；請先 cargo build --release --workspace 並執行 scripts/fetch-dict.sh" }
}

Write-Host "整理檔案：$stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage, "$stage\dict" | Out-Null
Copy-Item "$release\zct_tsf.dll", "$release\zct-service.exe", "$release\zct-settings.exe" $stage
Copy-Item "$dict\*" "$stage\dict"
Copy-Item (Join-Path $root 'LICENSE-MIT'), (Join-Path $root 'LICENSE-APACHE') $stage
Copy-Item (Join-Path $root 'installer\THIRD-PARTY-NOTICES.txt') $stage

Write-Host "建置安裝檔：版本 $version"
& $Iscc /Q "/DVersion=$version" "/DStage=$stage" (Join-Path $root 'installer\zoo-type.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC 失敗（$LASTEXITCODE）" }

$setup = Join-Path $root "target\installer\zoo-type-$version-windows-x64-setup.exe"
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
Set-Content -Path "$setup.sha256" -Value "$hash  $(Split-Path -Leaf $setup)" -Encoding ASCII
Write-Host "完成：$setup"
Write-Host "SHA256：$hash"
