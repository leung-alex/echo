//! Runtime visual values. The compiled UI depends on their interface, not defaults.
use std::collections::BTreeMap;

#[rustfmt::skip]
#[path = "style_defaults.rs"]
mod generated;
pub(crate) use generated::apply_style;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StyleValue {
    Color(u32, u32),
    Length(f32),
    Integer(i32),
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StyleSnapshot(BTreeMap<String, StyleValue>);
impl Default for StyleSnapshot {
    fn default() -> Self {
        generated::defaults()
    }
}
impl StyleSnapshot {
    pub fn color(&self, name: &str, dark: bool) -> slint::Color {
        let StyleValue::Color(light, night) = self.0[name] else {
            unreachable!()
        };
        let rgba = if dark { night } else { light };
        slint::Color::from_argb_u8(
            rgba as u8,
            (rgba >> 24) as u8,
            (rgba >> 16) as u8,
            (rgba >> 8) as u8,
        )
    }
    pub fn length(&self, name: &str) -> f32 {
        let StyleValue::Length(value) = self.0[name] else {
            unreachable!()
        };
        value
    }
    pub fn integer(&self, name: &str) -> i32 {
        let StyleValue::Integer(value) = self.0[name] else {
            unreachable!()
        };
        value
    }
    pub fn match_color(&self, dark: bool) -> String {
        let color = self.color("color.accent-text", dark);
        let mut value = format!(
            "#{:02x}{:02x}{:02x}",
            color.red(),
            color.green(),
            color.blue()
        );
        if color.alpha() != 255 {
            use std::fmt::Write;
            let _ = write!(value, "{:02x}", color.alpha());
        }
        value
    }

    #[cfg(any(debug_assertions, test))]
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct Document {
            #[serde(rename = "schemaVersion")]
            version: u32,
            tokens: BTreeMap<String, Token>,
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Token {
            #[serde(rename = "type")]
            kind: String,
            value: serde_json::Value,
            #[serde(default)]
            dark: String,
            #[serde(default)]
            runtime: bool,
        }
        let doc: Document = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        if doc.version != 1 {
            return Err("unsupported style schemaVersion".into());
        }
        if doc.tokens.len() != generated::SCHEMA.len() {
            return Err("token set changed; rebuild Echo".into());
        }
        for &(name, kind, runtime) in generated::SCHEMA {
            let token = doc
                .tokens
                .get(name)
                .ok_or_else(|| format!("missing {name}; rebuild Echo"))?;
            if token.kind != kind || token.runtime != runtime {
                return Err(format!("{name}: interface changed; rebuild Echo"));
            }
            let valid = match kind {
                "color" => {
                    token.value.as_str().and_then(parse_color).is_some()
                        && (token.dark.is_empty() || parse_color(&token.dark).is_some())
                }
                "string" => token
                    .value
                    .as_str()
                    .is_some_and(|s| !s.contains(['\r', '\n', '\0'])),
                "integer" => token.value.as_u64().is_some(),
                "length" | "number" | "angle" | "duration" => {
                    token.value.as_f64().is_some_and(f64::is_finite)
                }
                _ => false,
            };
            if !valid {
                return Err(format!("{name}: invalid {kind} value"));
            }
        }
        let mut next = Self::default();
        // An interface change needs regeneration and compilation, never a partial update.
        if doc.tokens.iter().filter(|(_, t)| t.runtime).count() != next.0.len() {
            return Err("runtime token set changed; rebuild Echo".into());
        }
        for (name, value) in &mut next.0 {
            let token = doc
                .tokens
                .get(name)
                .ok_or_else(|| format!("missing {name}; rebuild Echo"))?;
            if !token.runtime {
                return Err(format!("{name}: runtime flag changed; rebuild Echo"));
            }
            let invalid = || format!("{name}: invalid {} value", token.kind);
            *value = match (&*value, token.kind.as_str()) {
                (StyleValue::Color(..), "color") => {
                    let light = token.value.as_str().ok_or_else(invalid)?;
                    let dark = if token.dark.is_empty() {
                        light
                    } else {
                        &token.dark
                    };
                    StyleValue::Color(
                        parse_color(light).ok_or_else(invalid)?,
                        parse_color(dark).ok_or_else(invalid)?,
                    )
                }
                (StyleValue::Length(..), "length") | (StyleValue::Integer(..), "integer") => {
                    let n = token.value.as_f64().ok_or_else(invalid)?;
                    if !token.dark.is_empty()
                        || !n.is_finite()
                        || !(0.0..=4096.0).contains(&n)
                        || (name.starts_with("font.") && n < 1.0)
                    {
                        return Err(invalid());
                    }
                    if token.kind == "integer" {
                        if n.fract() != 0.0 || n > 1000.0 {
                            return Err(invalid());
                        }
                        StyleValue::Integer(n as i32)
                    } else {
                        StyleValue::Length(n as f32)
                    }
                }
                _ => return Err(format!("{name}: type changed; rebuild Echo")),
            };
        }
        Ok(next)
    }
}
#[cfg(any(debug_assertions, test))]
fn parse_color(value: &str) -> Option<u32> {
    if !value.starts_with('#')
        || !matches!(value.len(), 7 | 9)
        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let n = u32::from_str_radix(&value[1..], 16).ok()?;
    Some(if value.len() == 7 { (n << 8) | 255 } else { n })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../design/tokens/echo.tokens.json")).unwrap()
    }
    fn parse(value: &serde_json::Value) -> Result<StyleSnapshot, String> {
        StyleSnapshot::parse(&serde_json::to_vec(value).unwrap())
    }
    #[test]
    fn embedded_values_match_authoritative_source() {
        assert_eq!(parse(&source()).unwrap(), StyleSnapshot::default());
    }
    #[test]
    fn complete_update_preserves_types_and_alpha() {
        let mut doc = source();
        doc["tokens"]["color.row-text"]["value"] = "#12345680".into();
        doc["tokens"]["font.body"]["value"] = 18.into();
        doc["tokens"]["color.accent-text"]["value"] = "#12345680".into();
        let next = parse(&doc).unwrap();
        assert_eq!(next.color("color.row-text", false).alpha(), 128);
        assert_eq!(next.length("font.body"), 18.0);
        assert_eq!(next.match_color(false), "#12345680");
        assert_eq!(
            next.color("color.row-text", true),
            StyleSnapshot::default().color("color.row-text", true)
        );
    }
    #[test]
    fn invalid_field_or_interface_rejects_whole_update() {
        for (name, field, value) in [
            ("color.row-text", "value", serde_json::json!("#oops")),
            ("font.body", "value", serde_json::json!(-1)),
            ("font.body", "value", serde_json::json!(0)),
            ("row.radius", "value", serde_json::json!(999999)),
            ("font.weight-normal", "value", serde_json::json!(450.5)),
            ("font.body", "type", serde_json::json!("integer")),
            ("font.body", "runtime", serde_json::json!(false)),
            ("window.width", "value", serde_json::json!("oops")),
        ] {
            let mut doc = source();
            doc["tokens"][name][field] = value;
            assert!(parse(&doc).unwrap_err().contains(name));
        }
        let mut doc = source();
        doc["tokens"].as_object_mut().unwrap().remove("font.body");
        assert!(parse(&doc).is_err());
    }
    #[cfg(all(windows, debug_assertions))]
    #[test]
    fn watcher_recovers_and_stops_without_duplicate_updates() {
        use crate::events::{Event, Hub};
        use std::{
            sync::Arc,
            thread,
            time::{Duration, Instant},
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("styles.json");
        let mut doc = source();
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let hub = Arc::new(Hub::default());
        let (initial, watcher) = development::start_path(hub.clone(), path.clone());
        assert_eq!(initial, StyleSnapshot::default());
        std::fs::write(&path, b"{").unwrap();
        thread::sleep(Duration::from_millis(650));
        assert!(hub.take_test_events().is_empty());
        doc["tokens"]["color.row-text"]["value"] = "#777777".into();
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let start = Instant::now();
        loop {
            if let Some(Event::Styles(s)) = hub.take_test_events().pop() {
                assert_eq!(s, parse(&doc).unwrap());
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        thread::sleep(Duration::from_millis(650));
        assert!(hub.take_test_events().is_empty());
        let start = Instant::now();
        drop(watcher);
        assert!(start.elapsed() < Duration::from_millis(250));
        hub.close();
    }
}

#[cfg(all(windows, debug_assertions))]
pub(crate) mod development {
    use super::StyleSnapshot;
    use crate::events::{Event, Hub};
    use std::{
        path::PathBuf,
        sync::{mpsc, Arc},
        thread,
        time::Duration,
    };

    pub struct Watcher {
        stop: mpsc::Sender<()>,
        thread: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Watcher {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
    // Read and validate before creating the UI. No Slint handle crosses this boundary.
    pub fn start(hub: Arc<Hub>) -> (StyleSnapshot, Option<Watcher>) {
        let Some(path) = std::env::var_os("ECHO_DEV_STYLE_SOURCE").map(PathBuf::from) else {
            return (StyleSnapshot::default(), None);
        };
        if !path.is_absolute() {
            eprintln!("[Echo styles] source must be absolute");
            return (StyleSnapshot::default(), None);
        }
        start_path(hub, path)
    }
    pub(super) fn start_path(hub: Arc<Hub>, path: PathBuf) -> (StyleSnapshot, Option<Watcher>) {
        let initial = std::fs::read(&path);
        let snapshot = initial
            .as_ref()
            .map_err(|e| e.to_string())
            .and_then(|bytes| StyleSnapshot::parse(bytes))
            .unwrap_or_else(|error| {
                eprintln!("[Echo styles] {error}; retaining embedded defaults");
                StyleSnapshot::default()
            });
        let active = snapshot.clone();
        let (stop, receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut last_bytes = initial.ok();
            let mut active = active;
            let mut last_error = String::new();
            while receiver.recv_timeout(Duration::from_millis(500))
                == Err(mpsc::RecvTimeoutError::Timeout)
            {
                let result = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        if last_bytes.as_ref() == Some(&bytes) {
                            return Ok(None);
                        }
                        last_bytes = Some(bytes.clone());
                        StyleSnapshot::parse(&bytes).map(Some)
                    });
                match result {
                    Ok(Some(next)) => {
                        last_error.clear();
                        if next != active {
                            active = next.clone();
                            hub.post(Event::Styles(next));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if error != last_error {
                            eprintln!("[Echo styles] {error}; retaining last valid styles");
                            last_error = error;
                        }
                    }
                }
            }
        });
        (
            snapshot,
            Some(Watcher {
                stop,
                thread: Some(thread),
            }),
        )
    }
}
