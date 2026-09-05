fn main() {
    let configuration = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app-window.slint", configuration)
        .expect("compile the native Echo interface");
    println!("cargo:rerun-if-changed=resources/echo.rc");
    println!("cargo:rerun-if-changed=resources/echo.manifest");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("resources/echo.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile Echo's Windows manifest and icon");
    }
}
