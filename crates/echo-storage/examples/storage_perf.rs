use std::fs;
use std::path::{Path, PathBuf};

use echo_engine::{
    fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SourceContext,
};
use echo_storage::ClipboardStore;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize)]
struct PerfReport {
    schema: &'static str,
    scenarios: Scenarios,
}

#[derive(Serialize)]
struct Scenarios {
    history_query_5000_text: HistoryQueryScenario,
    mixed_200_rows_50_images: MixedRowsScenario,
    duplicate_large_image: DuplicateScenario,
    gc_referenced_and_orphan_blobs: GcScenario,
}

#[derive(Serialize)]
struct HistoryQueryScenario {
    entries: usize,
    pages: usize,
    exact_matches: usize,
}

#[derive(Serialize)]
struct MixedRowsScenario {
    entries: usize,
    image_entries: usize,
}

#[derive(Serialize)]
struct DuplicateScenario {
    first_duplicate: bool,
    second_duplicate: bool,
    same_id: bool,
    blob_files: usize,
}

#[derive(Serialize)]
struct GcScenario {
    referenced_blob_preserved: bool,
    orphan_blob_removed: bool,
}

fn main() {
    let root = PathBuf::from(
        std::env::var_os("ECHO_PERF_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".local/echo/perf")),
    );
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create perf root");
    let report = run(&root).expect("storage perf diagnostic");
    let _ = fs::remove_dir_all(&root);
    println!(
        "{}",
        serde_json::to_string(&PerfReport {
            schema: "echo.storage.perf.v1",
            scenarios: report,
        })
        .expect("serialize perf report")
    );
}

fn run(root: &Path) -> Result<Scenarios, String> {
    let history_root = root.join("history");
    let mut history = ClipboardStore::open(&history_root).map_err(|error| error.to_string())?;
    for sequence in 0..5_000 {
        history
            .record_capture(text_capture(
                &format!("echo-perf-text-{sequence:04}"),
                sequence as u64,
            ))
            .map_err(|error| error.to_string())?;
    }
    let exact_matches = history
        .list_entries("echo-perf-text-4999", 100)
        .map_err(|error| error.to_string())?
        .len();
    let mut pages = 0;
    let mut entries = 0;
    let mut cursor = None;
    loop {
        let page = history
            .list_entries_page("", 100, cursor)
            .map_err(|error| error.to_string())?;
        entries += page.items.len();
        pages += 1;
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    if entries != 5_000 || exact_matches != 1 || pages != 50 {
        return Err(format!(
            "history invariant failed: entries={entries} pages={pages} exact_matches={exact_matches}"
        ));
    }
    drop(history);

    let mixed_root = root.join("mixed");
    let mut mixed = ClipboardStore::open(&mixed_root).map_err(|error| error.to_string())?;
    for sequence in 0..200 {
        let is_image = sequence < 50;
        mixed
            .record_capture(if is_image {
                image_capture(sequence as u64)
            } else {
                text_capture(&format!("echo-perf-mixed-{sequence:03}"), sequence as u64)
            })
            .map_err(|error| error.to_string())?;
    }
    let (mixed_entries, image_entries) = count_entries(&mixed)?;
    if mixed_entries != 200 || image_entries != 50 {
        return Err(format!(
            "mixed invariant failed: entries={mixed_entries} image_entries={image_entries}"
        ));
    }
    drop(mixed);

    let duplicate_root = root.join("duplicate");
    let mut duplicate = ClipboardStore::open(&duplicate_root).map_err(|error| error.to_string())?;
    let large = large_image_capture(1);
    let first = duplicate
        .record_capture(large.clone())
        .map_err(|error| error.to_string())?;
    let second = duplicate
        .record_capture(large)
        .map_err(|error| error.to_string())?;
    let blob_files = fs::read_dir(duplicate_root.join("blobs"))
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .count();
    if first.duplicate || !second.duplicate || first.id != second.id || blob_files != 1 {
        return Err(format!(
            "duplicate invariant failed: first_duplicate={} second_duplicate={} same_id={} blob_files={blob_files}",
            first.duplicate,
            second.duplicate,
            first.id == second.id
        ));
    }
    drop(duplicate);

    let gc_root = root.join("gc");
    let mut gc = ClipboardStore::open(&gc_root).map_err(|error| error.to_string())?;
    let referenced = gc
        .record_capture(large_image_capture(2))
        .map_err(|error| error.to_string())?;
    let referenced_hash = gc
        .entry(referenced.id)
        .map_err(|error| error.to_string())?
        .and_then(|entry| {
            entry
                .representations
                .first()
                .and_then(|rep| rep.blob_hash.clone())
        })
        .ok_or_else(|| "referenced blob was not stored".to_owned())?;
    let orphan_bytes = vec![0xA5; 70_000];
    let mut hasher = Sha256::new();
    hasher.update(&orphan_bytes);
    let orphan_hash = format!("{:x}", hasher.finalize());
    fs::write(gc_root.join("blobs").join(&orphan_hash), orphan_bytes)
        .map_err(|error| error.to_string())?;
    gc.reconcile_blob_store()
        .map_err(|error| error.to_string())?;
    let referenced_blob_preserved = gc_root.join("blobs").join(&referenced_hash).is_file();
    let orphan_blob_removed = !gc_root.join("blobs").join(&orphan_hash).exists();
    if !referenced_blob_preserved || !orphan_blob_removed {
        return Err("GC invariant failed".to_owned());
    }

    Ok(Scenarios {
        history_query_5000_text: HistoryQueryScenario {
            entries,
            pages,
            exact_matches,
        },
        mixed_200_rows_50_images: MixedRowsScenario {
            entries: mixed_entries,
            image_entries,
        },
        duplicate_large_image: DuplicateScenario {
            first_duplicate: false,
            second_duplicate: true,
            same_id: true,
            blob_files: 1,
        },
        gc_referenced_and_orphan_blobs: GcScenario {
            referenced_blob_preserved,
            orphan_blob_removed,
        },
    })
}

fn count_entries(store: &ClipboardStore) -> Result<(usize, usize), String> {
    let mut total = 0;
    let mut images = 0;
    let mut cursor = None;
    loop {
        let page = store
            .list_entries_page("", 100, cursor)
            .map_err(|error| error.to_string())?;
        total += page.items.len();
        images += page
            .items
            .iter()
            .filter(|entry| entry.content_type == "image")
            .count();
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok((total, images));
        }
    }
}

fn text_capture(text: &str, sequence: u64) -> NormalizedCapture {
    let representation = ClipboardRepresentation {
        format: "text".to_owned(),
        mime_type: "text/plain;charset=utf-8".to_owned(),
        bytes: text.as_bytes().to_vec(),
    };
    NormalizedCapture {
        sequence,
        source: SourceContext::default(),
        content_type: ContentType::Text,
        preview_text: Some(text.to_owned()),
        searchable_text: Some(text.to_owned()),
        sanitized_html: None,
        fingerprint: fingerprint(std::slice::from_ref(&representation)),
        representations: vec![representation],
    }
}

fn image_capture(sequence: u64) -> NormalizedCapture {
    let bytes = vec![sequence as u8, 0x22, 0x33, 0x44];
    let representation = ClipboardRepresentation {
        format: "image".to_owned(),
        mime_type: "image/png".to_owned(),
        bytes,
    };
    NormalizedCapture {
        sequence,
        source: SourceContext::default(),
        content_type: ContentType::Image,
        preview_text: Some(format!("echo-perf-image-{sequence:03}")),
        searchable_text: Some(format!("echo-perf-image-{sequence:03}")),
        sanitized_html: None,
        fingerprint: fingerprint(std::slice::from_ref(&representation)),
        representations: vec![representation],
    }
}

fn large_image_capture(sequence: u64) -> NormalizedCapture {
    let bytes = (0..70_000usize)
        .map(|index| index.wrapping_add(sequence as usize) as u8)
        .collect::<Vec<_>>();
    let representation = ClipboardRepresentation {
        format: "image".to_owned(),
        mime_type: "image/png".to_owned(),
        bytes,
    };
    NormalizedCapture {
        sequence,
        source: SourceContext::default(),
        content_type: ContentType::Image,
        preview_text: Some(format!("r5-perf-large-image-{sequence}")),
        searchable_text: Some(format!("r5-perf-large-image-{sequence}")),
        sanitized_html: None,
        fingerprint: fingerprint(std::slice::from_ref(&representation)),
        representations: vec![representation],
    }
}
