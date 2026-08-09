use std::io::Cursor;

use image::{imageops, GrayImage, RgbImage, RgbaImage};
use jpeg_decoder::{Decoder, PixelFormat};
use jpeg_encoder::{ColorType, Encoder};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("JPEG decode failed: {0}")]
    Decode(String),
    #[error("decoded dimensions do not match the PDF image dictionary")]
    DimensionMismatch,
    #[error("decoded colour representation does not match the PDF colour space")]
    UnsupportedColour,
    #[error("image buffer dimensions are invalid")]
    InvalidBuffer,
    #[error("JPEG encode failed: {0}")]
    Encode(String),
}

/// Resize a direct JPEG stream with deterministic Lanczos3 and encode it back to JPEG.
/// Four-channel CMYK data is resized channel-for-channel; it is never converted through RGB.
pub fn resize_jpeg(
    encoded: &[u8],
    source: [u32; 2],
    target: [u32; 2],
    colour_space: &str,
) -> Result<Vec<u8>, TransformError> {
    let mut decoder = Decoder::new(Cursor::new(encoded));
    let pixels = decoder
        .decode()
        .map_err(|error| TransformError::Decode(error.to_string()))?;
    let info = decoder
        .info()
        .ok_or_else(|| TransformError::Decode("missing JPEG metadata".into()))?;
    if u32::from(info.width) != source[0] || u32::from(info.height) != source[1] {
        return Err(TransformError::DimensionMismatch);
    }
    let (resized, colour_type) = match (colour_space, info.pixel_format) {
        ("DeviceRGB", PixelFormat::RGB24) => {
            let image = RgbImage::from_raw(source[0], source[1], pixels)
                .ok_or(TransformError::InvalidBuffer)?;
            (
                imageops::resize(&image, target[0], target[1], imageops::FilterType::Lanczos3)
                    .into_raw(),
                ColorType::Rgb,
            )
        }
        ("DeviceGray", PixelFormat::L8) => {
            let image = GrayImage::from_raw(source[0], source[1], pixels)
                .ok_or(TransformError::InvalidBuffer)?;
            (
                imageops::resize(&image, target[0], target[1], imageops::FilterType::Lanczos3)
                    .into_raw(),
                ColorType::Luma,
            )
        }
        ("DeviceCMYK", PixelFormat::CMYK32) => {
            // RgbaImage is only a four-channel container here; no alpha compositing occurs.
            let image = RgbaImage::from_raw(source[0], source[1], pixels)
                .ok_or(TransformError::InvalidBuffer)?;
            (
                imageops::resize(&image, target[0], target[1], imageops::FilterType::Lanczos3)
                    .into_raw(),
                ColorType::Cmyk,
            )
        }
        _ => return Err(TransformError::UnsupportedColour),
    };
    let mut output = Vec::new();
    Encoder::new(&mut output, 90)
        .encode(&resized, target[0] as u16, target[1] as u16, colour_type)
        .map_err(|error| TransformError::Encode(error.to_string()))?;
    Ok(output)
}
