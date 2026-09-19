use std::{env, fs, path::Path};

// Resource changes rebuild this data crate, not Slint's generated component tree.
fn main() {
    println!("cargo:rerun-if-changed=catalog.json");
    println!("cargo:rerun-if-changed=../ui/lucide-icons");
    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read("catalog.json").unwrap()).unwrap();
    let mut bytes = Vec::new();
    let mut rust = String::from("static CATALOG: &[Entry] = &[\n");
    let mut previous = String::new();
    for icon in catalog.as_array().expect("icon catalog array") {
        let key = icon["key"].as_str().unwrap();
        let stem = key.strip_prefix("lucide-").expect("namespaced icon key");
        assert!(stem
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'));
        assert!(previous.as_str() < key, "catalog must be sorted and unique");
        previous = key.to_owned();
        let start = bytes.len();
        bytes.extend(fs::read(format!("../ui/lucide-icons/{stem}.svg")).unwrap());
        let end = bytes.len();
        let terms = icon["terms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|term| format!("{:?}", term.as_str().unwrap()))
            .collect::<Vec<_>>()
            .join(",");
        rust.push_str(&format!(
            "Entry {{ key: {key:?}, label: {:?}, terms: &[{terms}], start: {start}, end: {end} }},\n",
            icon["label"].as_str().unwrap()
        ));
    }
    rust.push_str("];\n");
    let out = env::var_os("OUT_DIR").unwrap();
    write_changed(&Path::new(&out).join("catalog.rs"), rust.as_bytes());
    write_changed(&Path::new(&out).join("icons.bin"), &bytes);

    // Preserve legacy keys and original bytes without compiling a Slint image array.
    println!("cargo:rerun-if-changed=system-catalog.json");
    println!("cargo:rerun-if-changed=../ui/favorite-icons");
    let catalog: serde_json::Value =
        serde_json::from_slice(&fs::read("system-catalog.json").unwrap()).unwrap();
    let mut keys = catalog
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["key"].as_str().unwrap())
        .filter(|key| *key != "none")
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert!(keys.windows(2).all(|p| p[0] < p[1]), "unique system keys");
    let mut bytes = Vec::new();
    let mut rust = String::from("static SYSTEM_CATALOG: &[SystemEntry] = &[\n");
    for key in keys {
        assert!(key.bytes().all(|c| c.is_ascii_alphanumeric()));
        let start = bytes.len();
        bytes.extend(fs::read(format!("../ui/favorite-icons/{key}.svg")).unwrap());
        let end = bytes.len();
        rust.push_str(&format!(
            "SystemEntry {{ key: {key:?}, start: {start}, end: {end} }},\n"
        ));
    }
    rust.push_str("];\n");
    write_changed(&Path::new(&out).join("system_catalog.rs"), rust.as_bytes());
    write_changed(&Path::new(&out).join("system_icons.bin"), &bytes);
}

fn write_changed(path: &Path, bytes: &[u8]) {
    if fs::read(path).ok().as_deref() != Some(bytes) {
        fs::write(path, bytes).unwrap();
    }
}
