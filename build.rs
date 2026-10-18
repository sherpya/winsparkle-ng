fn main() {
    for file in ["resources", "include/winsparkle.h"] {
        println!("cargo:rerun-if-changed={file}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/winsparkle.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
