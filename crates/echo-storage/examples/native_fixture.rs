//! Deterministic synthetic UI/performance fixtures. Refuses to overwrite data.
use echo_engine::{
    fingerprint, ClipboardRepresentation, ContentType, FavoriteDraft, NormalizedCapture,
    SourceContext,
};
use echo_storage::ClipboardStore;
use std::{io::Cursor, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("fixture output root required")?,
    );
    if root.exists() {
        return Err("refusing to overwrite existing fixtures".into());
    }
    std::fs::create_dir_all(&root)?;
    for (name, count, favorites, mixed) in [
        ("D0", 16_u64, 8_u64, true),
        ("D1", 5000, 200, false),
        ("D2", 200, 20, true),
    ] {
        let dir = root.join(name);
        let mut store = ClipboardStore::open(&dir)?;
        for i in 0..count {
            let text = match i % 7 {
                0 => format!("Echo fixture {i:04} · Project notes\nKeep history searchable and insert content without leaving the composer.\nUnicode: 中文输入 · café · こんにちは · 📝"),
                1 => format!("echo-fixture-{i:04}@example.net"),
                2 => format!("https://example.net/echo/design/{i:04}"),
                3 => format!("SELECT id, updated_at\nFROM clipboard_entries\nORDER BY updated_at DESC;\n-- fixture {i:04}"),
                _ => format!("echo-perf-text-{i:04} — Reusable content, available when you need it."),
            };
            let is_image = mixed && i % 4 == 0;
            let (bytes, format, mime, kind) = if is_image {
                let image = image::RgbaImage::from_fn(1280, 720, |x, y| {
                    image::Rgba([
                        ((u64::from(x) / 8 + i * 7) % 255) as u8,
                        ((u64::from(y) / 4 + i * 11) % 255) as u8,
                        (110 + i % 90) as u8,
                        255,
                    ])
                });
                let mut bytes = Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(image)
                    .write_to(&mut bytes, image::ImageOutputFormat::Png)?;
                (bytes.into_inner(), "image", "image/png", ContentType::Image)
            } else {
                (
                    text.as_bytes().to_vec(),
                    "text",
                    "text/plain;charset=utf-8",
                    ContentType::Text,
                )
            };
            let rep = ClipboardRepresentation {
                format: format.into(),
                mime_type: mime.into(),
                bytes,
            };
            store.record_capture(NormalizedCapture {
                sequence: i + 1,
                source: SourceContext::default(),
                content_type: kind,
                preview_text: Some(if is_image {
                    format!("Fixture image {i:04} · 1280 × 720")
                } else {
                    text.clone()
                }),
                searchable_text: Some(text),
                sanitized_html: None,
                fingerprint: fingerprint(std::slice::from_ref(&rep)),
                representations: vec![rep],
            })?;
        }
        for i in 0..favorites {
            store.create_favorite(FavoriteDraft {
                content: format!("Echo favorite {i:03}\nReusable test content — 中文与 Unicode ✓"),
                name: Some(match i % 4 {
                    0 => format!("Work email {i}"),
                    1 => format!("Project links {i}"),
                    2 => format!("Meeting notes {i}"),
                    _ => format!("Quick reply {i}"),
                }),
                icon_key: None,
                tags: vec!["fixture".into()],
            })?;
        }
        store.pin_history(count as i64)?;
        let mut settings = store.settings()?;
        settings.history_enabled = false;
        store.update_settings(&settings)?;
        drop(store);
        let conn = rusqlite::Connection::open(dir.join("echo.sqlite3"))?;
        conn.execute("UPDATE clipboard_entries SET updated_at = 1788609600000 - ((? - id) / 5) * 3600000, created_at = 1788609600000 - ((? - id) / 5) * 3600000", [count as i64, count as i64])?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        std::fs::write(
            dir.join("synthetic-fixture.json"),
            serde_json::to_vec(
                &serde_json::json!({"dataset": name, "synthetic": true, "history": count, "favorites": favorites, "images": if mixed {count.div_ceil(4)} else {0}, "capture_enabled": false}),
            )?,
        )?;
        println!(
            "{}",
            serde_json::json!({"dataset": name, "history": count, "favorites": favorites, "images": if mixed {count.div_ceil(4)} else {0}, "source_sha": "0dc699e42d8d667e502938d71e33216f92513e5a", "synthetic": true})
        );
    }
    Ok(())
}
