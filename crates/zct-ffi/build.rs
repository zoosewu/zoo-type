fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-env-changed=ZCT_FFI_HEADER");
    let path = std::env::var_os("ZCT_FFI_HEADER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("zct.h")
        });
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    cbindgen::Builder::new()
        .with_crate(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .with_language(cbindgen::Language::C)
        .with_include_guard("ZCT_FFI_H")
        .generate()
        .expect("產生 Swift 使用的 C ABI header")
        .write_to_file(path);
}
