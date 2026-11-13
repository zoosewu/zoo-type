//! 把圖示與版本資訊編進 DLL：Windows 工作列只認 DLL / EXE 裡的圖示資源（RT_GROUP_ICON），不認獨立的 .ico 檔
//! 圖示是第 0 個圖示群組（註冊時 IconIndex = 0）。

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
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let (icon, name) = channel_icon_and_name("動物園輸入法");
        let rc = write_version_rc(&name, "zoo_type.dll", true, Some(&icon));
        embed_resource::compile_for_cdylib(rc, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
