use super::screenshot::capture_raw;
use crate::output::CmdError;
use serde::Serialize;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

#[derive(Serialize)]
pub struct OcrMatch {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Finds `needle` on screen and returns its bounding box in absolute desktop
/// pixels. Matches at the line level (OCR only exposes word-level bounding
/// boxes, not per-substring), so the box returned is the first whole line
/// whose text contains `needle` - accurate for the common case of a short
/// UI label.
pub fn find(
    needle: &str,
    monitor: Option<&str>,
    window: Option<&str>,
    region: Option<&str>,
) -> Result<OcrMatch, CmdError> {
    let needle_lower = needle.to_lowercase();
    read(monitor, window, region)?
        .into_iter()
        .find(|line| line.text.to_lowercase().contains(&needle_lower))
        .ok_or_else(|| CmdError::not_found(format!("no text matching '{needle}' found on screen")))
}

/// Every line of text in the area via `Windows.Media.Ocr`, with its bounding
/// box in absolute desktop pixels: the union of its words.
pub fn read(monitor: Option<&str>, window: Option<&str>, region: Option<&str>) -> Result<Vec<OcrMatch>, CmdError> {
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| CmdError::unsupported(format!("no OCR language pack installed: {e}")))?;

    let (img, origin_x, origin_y) = capture_raw(monitor, window, region)?;
    let (width, height) = img.dimensions();

    // RgbaImage stores R,G,B,A per pixel; SoftwareBitmap's Bgra8 format wants B,G,R,A.
    let mut bgra = img.into_raw();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }

    let writer =
        DataWriter::new().map_err(|e| CmdError::os(format!("could not create a pixel buffer: {e}")))?;
    writer
        .WriteBytes(&bgra)
        .map_err(|e| CmdError::os(format!("could not write pixel data: {e}")))?;
    let buffer = writer
        .DetachBuffer()
        .map_err(|e| CmdError::os(format!("could not detach pixel buffer: {e}")))?;

    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
        BitmapAlphaMode::Ignore,
    )
    .map_err(|e| CmdError::os(format!("could not build a bitmap for OCR: {e}")))?;

    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| CmdError::os(format!("OCR recognition failed: {e}")))?
        .get()
        .map_err(|e| CmdError::os(format!("OCR recognition failed: {e}")))?;

    let mut out = Vec::new();
    let lines = result.Lines().map_err(|e| CmdError::os(e.to_string()))?;
    for line in &lines {
        let line_text = line.Text().map_err(|e| CmdError::os(e.to_string()))?.to_string();

        let words = line.Words().map_err(|e| CmdError::os(e.to_string()))?;
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for word in &words {
            let r = word.BoundingRect().map_err(|e| CmdError::os(e.to_string()))?;
            min_x = min_x.min(r.X);
            min_y = min_y.min(r.Y);
            max_x = max_x.max(r.X + r.Width);
            max_y = max_y.max(r.Y + r.Height);
        }
        if min_x > max_x {
            continue; // line had no words despite its text; skip
        }

        out.push(OcrMatch {
            text: line_text,
            x: origin_x + min_x.round() as i32,
            y: origin_y + min_y.round() as i32,
            w: (max_x - min_x).round() as i32,
            h: (max_y - min_y).round() as i32,
        });
    }
    Ok(out)
}
