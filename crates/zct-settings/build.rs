//! Slint 介面，以及版本資訊與圖示（工作管理員顯示「動物園輸入法 設定」）

include!("../../res/version_rc.rs");

fn main() {
    // 開發建置帶除錯資訊：介面測試（i-slint-backend-testing）要靠它找元件
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/settings.slint", config).expect("編譯 Slint 介面失敗");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zct-tsf/res/zct.ico");
        println!("cargo:rerun-if-changed=../zct-tsf/res/zct.ico");
        let rc = write_version_rc(
            "動物園輸入法 設定",
            "zoo-type-settings.exe",
            false,
            Some(&icon),
        );
        embed_resource::compile(rc, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
