//! 把圖示編進 DLL：Windows 工作列只認 DLL / EXE 裡的圖示資源（RT_GROUP_ICON），不認獨立的 .ico 檔

fn main() {
    println!("cargo:rerun-if-changed=res/zct.rc");
    println!("cargo:rerun-if-changed=res/zct.ico");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        embed_resource::compile_for_cdylib("res/zct.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
