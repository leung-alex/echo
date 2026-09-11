use echo_engine::ClipboardRepresentation;

#[derive(Clone)]
pub(crate) enum InsertPayload {
    Text(String),
    Image((u32, usize, [u8; 32])),
}

impl InsertPayload {
    pub(super) fn retain(payload: &[ClipboardRepresentation]) -> Result<Self, String> {
        if let Some(image) = payload.iter().find(|r| r.format == "image") {
            return crate::windows_impl::inline_image_fingerprint(&image.bytes)
                .map(Self::Image)
                .ok_or_else(|| {
                    "Original image is not a supported BMP or PNG; nothing was replaced".into()
                });
        }
        let text = payload.iter().find(|r| r.format == "text")
            .ok_or("This item has no plain-text representation. Use Copy or manual copying; the query was not deleted.")?;
        let text = String::from_utf8(text.bytes.clone())
            .map_err(|_| "Original text is not valid UTF-8")?;
        if text.encode_utf16().count() > echo_engine::MAX_COMPOSER_UNITS || text.contains('\0') {
            return Err(
                "Item is too large for verified inline replacement; use manual copying".into(),
            );
        }
        Ok(Self::Text(text))
    }

    pub(super) fn clipboard_matches(&self, sequence: u64) -> bool {
        match self {
            Self::Text(text) => crate::windows_impl::validate_inline_clipboard(sequence, text),
            Self::Image(hash) => crate::windows_impl::validate_inline_image(sequence, hash),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_only_original_is_an_insert_payload() {
        let mut bmp = vec![0u8; 58];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&1i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        let payload = ClipboardRepresentation {
            format: "image".into(),
            mime_type: "image/bmp".into(),
            bytes: bmp,
        };
        assert!(InsertPayload::retain(&[payload]).is_ok());
    }
}
