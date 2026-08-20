use echo_engine::Library;
use echo_storage::SharedClipboardStore;
use tauri::http::{header, Response, StatusCode};

const PLACEHOLDER_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];

pub(crate) fn serve(library: &Library<SharedClipboardStore>, path: &str) -> Response<Vec<u8>> {
    let hash = path
        .strip_prefix("/thumbnail/")
        .and_then(|value| value.strip_suffix(".png"));
    let thumbnail = hash
        .and_then(|hash| library.store().read_thumbnail(hash).ok().flatten())
        .filter(|thumbnail| thumbnail.metadata.content_hash == hash.unwrap_or_default());
    let (bytes, content_hash, placeholder) = thumbnail
        .map(|thumbnail| (thumbnail.bytes, thumbnail.metadata.content_hash, false))
        .unwrap_or_else(|| (PLACEHOLDER_PNG.to_vec(), "placeholder".to_owned(), true));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::ETAG, format!("\"{content_hash}\""))
        .header(
            "X-Echo-Preview",
            if placeholder {
                "placeholder"
            } else {
                "thumbnail"
            },
        )
        .body(bytes)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}
