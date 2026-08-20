use base64::Engine as _;

use echo_engine::ClipboardRepresentation;

use crate::transport::ImagePreview;

pub(crate) fn image_preview(representation: ClipboardRepresentation) -> ImagePreview {
    ImagePreview {
        mime_type: representation.mime_type,
        base64: base64::engine::general_purpose::STANDARD.encode(representation.bytes),
    }
}
