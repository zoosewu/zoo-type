// 共用：產生 Windows 版本資訊資源（工作管理員「描述」欄、檔案內容的「詳細資料」）與圖示。
// 由各 crate 的 build.rs 以 include! 引入；版號取自 Cargo.toml。

/// 產生 .rc 到 OUT_DIR，回傳路徑
#[allow(dead_code)]
fn write_version_rc(description: &str, file_name: &str, is_dll: bool, icon: Option<&std::path::Path>) -> std::path::PathBuf {
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let mut nums = version.split(['.', '-', '+']).map(|p| p.parse::<u16>().unwrap_or(0));
    let (a, b, c) = (
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    );
    // rc 字串中的反斜線要跳脫
    let icon = icon
        .map(|p| format!("1 ICON \"{}\"\n", p.display().to_string().replace('\\', "\\\\")))
        .unwrap_or_default();
    let file_type = if is_dll { "0x2L" } else { "0x1L" };
    let rc = format!(
        r#"#pragma code_page(65001)
{icon}1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE {file_type}
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040404b0"
        BEGIN
            VALUE "CompanyName", "zoosewu"
            VALUE "FileDescription", "{description}"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "{file_name}"
            VALUE "OriginalFilename", "{file_name}"
            VALUE "ProductName", "動物園輸入法"
            VALUE "ProductVersion", "{version}"
            VALUE "LegalCopyright", "MIT OR Apache-2.0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0404, 1200
    END
END
"#
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("version.rc");
    std::fs::write(&out, rc).expect("寫入 version.rc");
    println!("cargo:rerun-if-changed=../../res/version_rc.rs");
    out
}
