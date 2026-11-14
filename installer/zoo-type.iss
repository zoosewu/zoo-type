; 動物園輸入法（ZCT）正式安裝程式
;
; 建置：scripts/build-installer.ps1（讀 Cargo.toml 的版號、整理要打包的檔案、呼叫 ISCC）
;
; 新舊版本並存：每個版本裝在 {app}\<版本>\，安裝步驟由共用的 scripts/install.ps1 處理
; （與開發版 install-dev.ps1 相同），已開著的 App 繼續用舊版，重開 App 後才換新版。

#ifndef Version
  #error 請用 scripts/build-installer.ps1 建置（需要 /DVersion=x.y.z）
#endif
#ifndef Stage
  #define Stage "..\target\installer\stage"
#endif

[Setup]
AppId={{5E2B7C1A-9D44-4F0B-8B6E-3A1C2D7F9E01}
AppName=動物園輸入法
AppVersion={#Version}
AppVerName=動物園輸入法 {#Version}
AppPublisher=zoosewu
AppPublisherURL=https://github.com/zoosewu/zoo-type
AppSupportURL=https://github.com/zoosewu/zoo-type/issues
AppUpdatesURL=https://github.com/zoosewu/zoo-type/releases
; 安裝位置固定（與開發版相同，可互相覆蓋）；改名前是 {autopf}\ZCT，不沿用舊位置
DefaultDirName={autopf}\ZooType
UsePreviousAppDir=no
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; 輸入法 DLL 幾乎被所有 App 載入；裝到新的版本資料夾、不覆蓋舊檔，所以不要求關閉程式
CloseApplications=no
RestartApplications=no
OutputDir=..\target\installer
OutputBaseFilename=zoo-type-{#Version}-windows-x64-setup
SetupIconFile=..\crates\zct-tsf\res\zct.ico
UninstallDisplayName=動物園輸入法
UninstallDisplayIcon={app}\{#Version}\zoo_type.dll
LicenseFile=..\LICENSE-MIT
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "zh_tw"; MessagesFile: "ChineseTraditional.isl"

[Files]
; 同一版重裝時 DLL 可能被載入中：排到重開機替換；解除安裝時使用中的檔案排到重開機刪除
Source: "{#Stage}\*"; DestDir: "{app}\{#Version}"; Flags: ignoreversion recursesubdirs createallsubdirs restartreplace uninsrestartdelete
Source: "..\scripts\install.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion
Source: "..\scripts\uninstall.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion

[UninstallRun]
; 檔案刪除前：從鍵盤清單移除、解除註冊、停止服務程序、刪除所有版本資料夾
Filename: "powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\scripts\uninstall.ps1"" -Base ""{app}"""; Flags: runhidden waituntilterminated; RunOnceId: "ZctUninstall"

[Code]
var
  DataPage: TInputDirWizardPage;
  { 精靈啟動時判斷一次，之後顯示頁面與執行安裝步驟都用同一個結果 }
  IsExistingUser: Boolean;

{ 已經用過（有 location.toml 或任何資料）：不再詢問，沿用原本的學習資料位置 }
function HasData(Dir: String): Boolean;
begin
  Result := FileExists(Dir + '\location.toml') or FileExists(Dir + '\config.toml') or DirExists(Dir + '\local');
end;

function ExistingUser: Boolean;
begin
  { 改名前的資料夾是 zct（服務程序第一次啟動時會搬到 ZooType） }
  Result := HasData(ExpandConstant('{userappdata}\ZooType')) or HasData(ExpandConstant('{userappdata}\zct'));
end;

procedure InitializeWizard;
begin
  IsExistingUser := ExistingUser;
  DataPage := CreateInputDirPage(wpLicense, '資料位置',
    '設定、詞庫與學習紀錄要存在哪裡？',
    '更新時沿用這個位置。',
    False, '');
  DataPage.Add('');
  DataPage.Values[0] := ExpandConstant('{userappdata}\ZooType');
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (PageID = DataPage.ID) and IsExistingUser;
end;

{ 執行共用的安裝核心；失敗時提示（靜默安裝時可用 /SUPPRESSMSGBOXES 略過） }
procedure RunPart(Part: String; AsOriginalUser: Boolean);
var
  Params: String;
  Code: Integer;
  Ok: Boolean;
begin
  Params := '-NoProfile -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\scripts\install.ps1') +
    '" -Dest "' + ExpandConstant('{app}\{#Version}') + '" -Part ' + Part;
  { 初次安裝才帶學習資料位置；更新時沿用 location.toml }
  if (Part = 'User') and not IsExistingUser then
    Params := Params + ' -DataRoot "' + DataPage.Values[0] + '"';
  if AsOriginalUser then
    Ok := ExecAsOriginalUser('powershell.exe', Params, '', SW_HIDE, ewWaitUntilTerminated, Code)
  else
    Ok := Exec('powershell.exe', Params, '', SW_HIDE, ewWaitUntilTerminated, Code);
  if (not Ok) or (Code <> 0) then
    SuppressibleMsgBox('安裝步驟「' + Part + '」失敗（代碼 ' + IntToStr(Code) + '）。', mbError, MB_OK, IDOK);
end;

{ 自動更新（zoo-type-updater 以 SYSTEM 執行）帶 /SYSTEMUPDATE：沒有使用者可以代表，略過使用者部分
  （鍵盤清單與資料位置沿用；登入自動啟動的路徑由服務程序啟動時自己更新） }
function IsSystemUpdate: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/SYSTEMUPDATE') = 0 then
      Result := True;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    { 系統部分：註冊輸入法、current.txt、自動更新的排程工作、清除舊版（系統管理員） }
    RunPart('System', False);
    { 使用者部分：鍵盤清單、開機自動啟動、學習資料位置（以安裝者本人的身分） }
    if not IsSystemUpdate then
      RunPart('User', True);
  end;
end;
