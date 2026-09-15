//! Bounded, display-sized previews. Original payloads remain unchanged.
use crate::events::PixelData;
use image::GenericImageView;

pub const CACHE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviewSize {
    pub width: u32,
    pub height: u32,
}
impl PreviewSize {
    pub fn physical(width: f32, height: f32, scale: f32) -> Self {
        let dimension = |value: f32| (value * scale).ceil().clamp(1., 1024.) as u32;
        Self {
            width: dimension(width),
            height: dimension(height),
        }
    }
    pub fn covers(self, other: Self) -> bool {
        self.width >= other.width && self.height >= other.height
    }
}

pub fn decode(bytes: &[u8], size: PreviewSize) -> Result<PixelData, String> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Preview source exceeds 32 MiB".into());
    }
    let make_reader = || {
        image::io::Reader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (width, height) = make_reader()?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
        return Err("Preview source exceeds 16 megapixels".into());
    }
    let mut reader = make_reader()?;
    let mut limits = image::io::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let source = reader.decode().map_err(|e| e.to_string())?;
    // Never enlarge a small original, and never resample from the 256px thumbnail.
    let resized = if width <= size.width && height <= size.height {
        source
    } else {
        source.resize(
            size.width.max(1),
            size.height.max(1),
            image::imageops::FilterType::Lanczos3,
        )
    };
    let (width, height) = resized.dimensions();
    Ok(PixelData {
        requested: size,
        width,
        height,
        rgba: resized.into_rgba8().into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_preserves_small_original_pixels_and_bounds_large_images() {
        let image =
            image::RgbaImage::from_fn(350, 35, |x, y| image::Rgba([x as u8, y as u8, 42, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut png, image::ImageOutputFormat::Png)
            .unwrap();
        let original = decode(png.get_ref(), PreviewSize::physical(500., 140., 1.)).unwrap();
        assert_eq!((original.width, original.height), (350, 35));
        assert_eq!(original.rgba, image.into_raw());
        let small = decode(png.get_ref(), PreviewSize::physical(175., 140., 1.)).unwrap();
        assert_eq!((small.width, small.height), (175, 18));
        assert_eq!(
            PreviewSize::physical(300., 140., 2.),
            PreviewSize {
                width: 600,
                height: 280
            }
        );
        assert!(decode(b"bad", PreviewSize::physical(100., 100., 1.)).is_err());
    }
}
