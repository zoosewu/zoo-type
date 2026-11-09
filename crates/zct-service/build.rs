//! 版本資訊與圖示（工作管理員顯示「動物園輸入法 服務程序」）

include!("../../res/version_rc.rs");

fn main() {
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zct-tsf/res/zct.ico");
        println!("cargo:rerun-if-changed=../zct-tsf/res/zct.ico");
        let rc = write_version_rc("動物園輸入法 服務程序", "zoo-type-service.exe", false, Some(&icon));
        embed_resource::compile(rc, embed_resource::NONE)
            .manifest_required()
            .unwrap();
    }
}
