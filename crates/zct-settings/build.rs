//! Slint 介面，以及版本資訊與圖示（工作管理員顯示「動物園輸入法 設定」）

include!("../../res/version_rc.rs");

/// 開發版（cargo feature `dev`）用帶「DEV」標籤的圖示，描述加上「開發版」
fn channel_icon_and_name(name: &str) -> (std::path::PathBuf, String) {
    let dev = std::env::var("CARGO_FEATURE_DEV").is_ok();
    let res = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zct-tsf/res");
    println!("cargo:rerun-if-changed=../zct-tsf/res/zct.ico");
    println!("cargo:rerun-if-changed=../zct-tsf/res/zct-dev.ico");
    let icon = res.join(if dev { "zct-dev.ico" } else { "zct.ico" });
    let name = match (dev, name.split_once(' ')) {
        (false, _) => name.to_owned(),
        (true, Some((app, what))) => format!("{app} 開發版 {what}"),
        (true, None) => format!("{name} 開發版"),
    };
    (icon, name)
}

fn main() {
    // 開發建置帶除錯資訊：介面測試（i-slint-backend-testing）要靠它找元件
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/settings.slint", config).expect("編譯 Slint 介面失敗");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let (icon, name) = channel_icon_and_name("動物園輸入法 設定");
        let rc = write_version_rc(&name, "zoo-type-settings.exe", false, Some(&icon));
        embed_resource::compile(rc, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
