//! 把圖示與版本資訊編進 DLL：Windows 工作列只認 DLL / EXE 裡的圖示資源（RT_GROUP_ICON），不認獨立的 .ico 檔
//! 圖示是第 0 個圖示群組（註冊時 IconIndex = 0）。

include!("../../res/version_rc.rs");

fn main() {
    println!("cargo:rerun-if-changed=res/zct.ico");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("res")
            .join("zct.ico");
        let rc = write_version_rc("動物園輸入法", "zoo_type.dll", true, Some(&icon));
        embed_resource::compile_for_cdylib(rc, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
