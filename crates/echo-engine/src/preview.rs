use std::io::Cursor;

use image::{DynamicImage, GenericImageView, ImageOutputFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ClipboardRepresentation, ContentIdentity};

pub const DEFAULT_THUMBNAIL_MAX_EDGE: u32 = 256;
pub const THUMBNAIL_MIME_TYPE: &str = "image/png";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub source_hash: String,
    pub content_hash: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewAsset {
    pub thumbnail: Thumbnail,
    pub bytes: Vec<u8>,
}

pub fn thumbnail_for_capture(
    representations: &[ClipboardRepresentation],
    identity: &ContentIdentity,
) -> Option<PreviewAsset> {
    representations
        .iter()
        .zip(identity.representations.iter())
        .find(|(representation, _)| {
            representation.format == "image" || representation.mime_type.starts_with("image/")
        })
        .and_then(|(representation, source)| {
            thumbnail_for_representation(representation, &source.hash)
        })
}

pub fn thumbnail_for_representation(
    representation: &ClipboardRepresentation,
    source_hash: &str,
) -> Option<PreviewAsset> {
    if !representation.mime_type.starts_with("image/") && representation.format != "image" {
        return None;
    }
    let decoded = image::load_from_memory(&representation.bytes).ok()?;
    let (width, height) = decoded.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let thumbnail = decoded.thumbnail(DEFAULT_THUMBNAIL_MAX_EDGE, DEFAULT_THUMBNAIL_MAX_EDGE);
    let (thumbnail_width, thumbnail_height) = thumbnail.dimensions();
    let bytes = encode_png(thumbnail)?;
    let content_hash = format!("{:x}", Sha256::digest(&bytes));
    Some(PreviewAsset {
        thumbnail: Thumbnail {
            source_hash: source_hash.to_owned(),
            content_hash,
            mime_type: THUMBNAIL_MIME_TYPE.to_owned(),
            width: thumbnail_width,
            height: thumbnail_height,
            byte_size: bytes.len() as u64,
        },
        bytes,
    })
}

fn encode_png(image: DynamicImage) -> Option<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageOutputFormat::Png).ok()?;
    Some(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_representation(width: u32, height: u32) -> ClipboardRepresentation {
        let image = DynamicImage::new_rgba8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageOutputFormat::Png).unwrap();
        ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: bytes.into_inner(),
        }
    }

    #[test]
    fn thumbnail_is_png_and_never_exceeds_the_default_edge() {
        let representation = png_representation(1024, 512);
        let identity = ContentIdentity::from_representations(std::slice::from_ref(&representation));
        let asset = thumbnail_for_capture(&[representation], &identity).unwrap();
        assert_eq!(asset.thumbnail.mime_type, THUMBNAIL_MIME_TYPE);
        assert!(asset.thumbnail.width <= DEFAULT_THUMBNAIL_MAX_EDGE);
        assert!(asset.thumbnail.height <= DEFAULT_THUMBNAIL_MAX_EDGE);
        assert_eq!(asset.thumbnail.byte_size, asset.bytes.len() as u64);
        assert_eq!(&asset.bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn invalid_image_does_not_make_capture_previewable() {
        let representation = ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/bmp".to_owned(),
            bytes: vec![1, 2, 3],
        };
        let identity = ContentIdentity::from_representations(std::slice::from_ref(&representation));
        assert!(thumbnail_for_capture(&[representation], &identity).is_none());
    }
}
