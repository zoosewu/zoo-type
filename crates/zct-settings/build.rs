//! Slint 介面，以及版本資訊與圖示（工作管理員顯示「動物園輸入法 設定」）

include!("../../res/version_rc.rs");

fn main() {
    slint_build::compile("ui/settings.slint").expect("編譯 Slint 介面失敗");
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
