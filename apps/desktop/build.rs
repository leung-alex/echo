fn main() {
    let catalog: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("i18n/zh-CN.json")).expect("read translation catalog");
    let translations =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("translations");
    let messages = translations.join("zh-CN/LC_MESSAGES");
    std::fs::create_dir_all(&messages).unwrap();
    let mut po = String::from("msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Language: zh-CN\\n\"\n\"Plural-Forms: nplurals=1; plural=0;\\n\"\n\n");
    for (id, translation) in &catalog {
        po.push_str(&format!(
            "msgid {}\nmsgstr {}\n\n",
            serde_json::to_string(id).unwrap(),
            serde_json::to_string(translation).unwrap()
        ));
    }
    std::fs::write(messages.join("echo-desktop.po"), po).unwrap();
    println!("cargo:rerun-if-changed=i18n/zh-CN.json");
    let configuration = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_default_translation_context(slint_build::DefaultTranslationContext::None)
        .with_bundled_translations(translations);
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
