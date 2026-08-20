use crate::ClipboardRepresentation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewAsset {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub fn image_asset(
    representations: impl IntoIterator<Item = ClipboardRepresentation>,
) -> Option<PreviewAsset> {
    representations
        .into_iter()
        .find(|representation| representation.mime_type.starts_with("image/"))
        .map(|representation| PreviewAsset {
            mime_type: representation.mime_type,
            bytes: representation.bytes,
        })
}
