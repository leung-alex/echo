//! Deterministic synthetic UI/performance fixtures. Refuses to overwrite data.
use echo_engine::{
    fingerprint, ClipboardRepresentation, ContentType, FavoriteDraft, NormalizedCapture,
    SourceContext, SpaceAction, SpaceCommand, SpaceDraft,
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
    let memory50 = std::env::args().nth(2).as_deref() == Some("--memory50");
    let software_deck = std::env::args().nth(2).as_deref() == Some("--software-deck");
    let datasets = if software_deck {
        vec![("T", 2000, 200, false), ("M", 2000, 200, true)]
    } else if memory50 {
        vec![
            ("S0", 0, 0, false),
            ("S1", 2000, 200, false),
            ("S2", 2000, 200, true),
        ]
    } else {
        vec![
            ("D0", 16_u64, 8_u64, true),
            ("D1", 5000, 200, false),
            ("D2", 200, 20, true),
        ]
    };
    for (name, count, favorites, mixed) in datasets {
        let dir = root.join(name);
        let mut store = ClipboardStore::open(&dir)?;
        for i in 0..count {
            let mut text = match i % 7 {
                0 => format!("Echo fixture {i:04} · Project notes\nKeep history searchable and insert content without leaving the composer.\nUnicode: 中文输入 · café · こんにちは · 📝"),
                1 => format!("echo-fixture-{i:04}@example.net"),
                2 => format!("https://example.net/echo/design/{i:04}"),
                3 => format!("SELECT id, updated_at\nFROM clipboard_entries\nORDER BY updated_at DESC;\n-- fixture {i:04}"),
                _ => format!("echo-perf-text-{i:04} — Reusable content, available when you need it."),
            };
            if memory50 && i % 100 == 99 {
                text.push_str(&"\nMemory50 long searchable Unicode content 中文 📝".repeat(200));
            }
            let is_image = mixed
                && if software_deck {
                    i >= 1800 && i % 10 == 8
                } else {
                    i % 4 == 0
                };
            let (image_width, image_height) = if software_deck {
                (1920, 1080)
            } else {
                (1280, 720)
            };
            let (bytes, format, mime, kind) = if is_image {
                let image = image::RgbaImage::from_fn(image_width, image_height, |x, y| {
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
                    format!("Fixture image {i:04} · {image_width} × {image_height}")
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
        if count > 0 {
            store.pin_history(count as i64)?;
        }
        if software_deck {
            for (index, title) in ["Work notes", "An empty space"].into_iter().enumerate() {
                let space = store
                    .apply_space_command(SpaceCommand {
                        space_id: None,
                        expected_revision: None,
                        request_id: format!("software-deck-space-{index}"),
                        action: SpaceAction::Create(SpaceDraft {
                            title: title.into(),
                            icon_key: Some("Folder".into()),
                            description: String::new(),
                        }),
                    })?
                    .created_space
                    .ok_or("Missing synthetic space")?;
                if index == 0 {
                    let ids = store
                        .list_saved_items_page("", 10, None)?
                        .items
                        .into_iter()
                        .map(|item| item.id)
                        .collect();
                    let revision = store
                        .list_spaces()?
                        .into_iter()
                        .find(|s| s.id == space)
                        .ok_or("Missing synthetic metadata")?
                        .revision;
                    store.apply_space_command(SpaceCommand {
                        space_id: Some(space),
                        expected_revision: Some(revision),
                        request_id: "software-deck-share-ten".into(),
                        action: SpaceAction::AddItems(ids),
                    })?;
                }
            }
        }
        if memory50 && mixed {
            let space = store
                .apply_space_command(SpaceCommand {
                    space_id: None,
                    expected_revision: None,
                    request_id: "memory50-create-space".into(),
                    action: SpaceAction::Create(SpaceDraft {
                        title: "Memory50 Work".into(),
                        icon_key: None,
                        description: String::new(),
                    }),
                })?
                .created_space
                .ok_or("fixture space was not created")?;
            store.create_favorite_in_space(
                FavoriteDraft {
                    content: "Memory50 custom space retained original content".into(),
                    name: Some("Memory50 custom item".into()),
                    icon_key: None,
                    tags: vec![],
                },
                space,
                None,
            )?;
        }
        // Authorized native-test runtime disables capture before starting its listener.
        drop(store);
        let conn = rusqlite::Connection::open(dir.join("echo.sqlite3"))?;
        conn.execute("UPDATE clipboard_entries SET updated_at = 1788609600000 - ((? - id) / 5) * 3600000, created_at = 1788609600000 - ((? - id) / 5) * 3600000", [count as i64, count as i64])?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        let history: i64 =
            conn.query_row("SELECT COUNT(*) FROM clipboard_entries", [], |r| r.get(0))?;
        let favorites: i64 =
            conn.query_row("SELECT COUNT(*) FROM saved_items", [], |r| r.get(0))?;
        let images: i64 = conn.query_row(
            "SELECT COUNT(*) FROM clipboard_entries WHERE content_type = 'image'",
            [],
            |r| r.get(0),
        )?;
        std::fs::write(
            dir.join("synthetic-fixture.json"),
            serde_json::to_vec(
                &serde_json::json!({"dataset": name, "synthetic": true, "history": history, "favorites": favorites, "images": images, "capture_enabled": false}),
            )?,
        )?;
        println!(
            "{}",
            serde_json::json!({"dataset": name, "history": history, "favorites": favorites, "images": images, "generator": "echo-storage/native_fixture", "synthetic": true})
        );
    }
    Ok(())
}
