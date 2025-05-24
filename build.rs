fn main() {
    let docs_builder = std::env::var("DOCS_RS").is_ok();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if (target_os == "windows") && !docs_builder {
        build_wrapper_wintun();
    }
}
fn build_wrapper_wintun() {
    use std::env;
    use std::path::PathBuf;
    let header_path = "src/platform/windows/tun/wintun_functions.h";

    println!("cargo:rerun-if-changed={}", header_path);

    let bindings = bindgen::Builder::default()
        .header(header_path)
        .allowlist_function("Wintun.*")
        .allowlist_type("WINTUN_.*")
        .dynamic_library_name("wintun")
        .dynamic_link_require_all(true)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings for wintun_functions.h");
    println!("OUT_DIR = {}", env::var("OUT_DIR").unwrap());
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
