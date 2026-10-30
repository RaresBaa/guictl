use crate::output::CmdError;
use image::RgbaImage;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_BITFIELDS, BI_RGB};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};

pub fn get_text() -> Result<String, CmdError> {
    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| CmdError::os(e.to_string()))?;
        let result = (|| -> Result<String, CmdError> {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32)
                .map_err(|e| CmdError::os(e.to_string()))?;
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(handle.0)) as *const u16;
            if ptr.is_null() {
                return Err(CmdError::os("clipboard data could not be locked"));
            }
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let text = String::from_utf16_lossy(slice);
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(handle.0));
            Ok(text)
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Reads a CF_DIB bitmap off the clipboard and saves it as a PNG at `path`.
/// Only uncompressed 24/32-bit BI_RGB and 32-bit BI_BITFIELDS are supported,
/// which covers the DIBs every common source (Print Screen, Snipping Tool,
/// Paint, browsers) puts on the clipboard.
pub fn get_image(path: &str) -> Result<(), CmdError> {
    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| CmdError::os(e.to_string()))?;
        let result = (|| -> Result<(), CmdError> {
            let handle =
                GetClipboardData(CF_DIB.0 as u32).map_err(|e| CmdError::os(e.to_string()))?;
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal) as *const u8;
            if ptr.is_null() {
                return Err(CmdError::os("clipboard data could not be locked"));
            }
            let size = GlobalSize(hglobal);
            let decode_result = decode_dib(ptr, size);
            let _ = GlobalUnlock(hglobal);
            let img = decode_result?;
            img.save(path).map_err(|e| CmdError::os(e.to_string()))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

unsafe fn decode_dib(ptr: *const u8, size: usize) -> Result<RgbaImage, CmdError> {
    if size < std::mem::size_of::<BITMAPINFOHEADER>() {
        return Err(CmdError::os("clipboard bitmap header is truncated"));
    }
    let header = &*(ptr as *const BITMAPINFOHEADER);
    // Masks saying which bits of a pixel hold red, green and blue. They sit
    // right after the 40-byte header, as part of the header in its longer forms.
    let header_size = header.biSize as usize;
    let masks = if header.biCompression == BI_BITFIELDS.0 {
        if header.biBitCount != 32 || size < 52 {
            return Err(CmdError::os(format!(
                "unsupported clipboard bitmap depth: {}-bit with bit masks",
                header.biBitCount
            )));
        }
        let read = |offset: usize| (ptr.add(offset) as *const u32).read_unaligned();
        let masks = [read(40), read(44), read(48)];
        if masks.iter().any(|m| m.count_ones() != 8) {
            return Err(CmdError::os("unsupported clipboard bitmap: its colors aren't 8 bits each"));
        }
        Some(masks)
    } else if header.biCompression == BI_RGB.0 {
        None
    } else {
        return Err(CmdError::os(
            "only uncompressed (BI_RGB or BI_BITFIELDS) clipboard bitmaps are supported",
        ));
    };
    let (bytes_per_pixel, has_alpha) = match header.biBitCount {
        24 => (3usize, false),
        32 => (4usize, true),
        n => {
            return Err(CmdError::os(format!(
                "unsupported clipboard bitmap depth: {n}-bit"
            )))
        }
    };

    let width = header.biWidth.unsigned_abs() as usize;
    let top_down = header.biHeight < 0;
    let height = header.biHeight.unsigned_abs() as usize;
    let stride = ((width * bytes_per_pixel + 3) / 4) * 4;
    let pixels_start = header_size + if masks.is_some() && header_size == 40 { 12 } else { 0 };
    if size < pixels_start + stride * height {
        return Err(CmdError::os("clipboard bitmap pixel data is truncated"));
    }
    let pixels = std::slice::from_raw_parts(ptr.add(pixels_start), stride * height);

    let mut img = RgbaImage::new(width as u32, height as u32);
    for y in 0..height {
        // DIB rows are bottom-up unless biHeight is negative.
        let src_row = if top_down { y } else { height - 1 - y };
        let row = &pixels[src_row * stride..src_row * stride + width * bytes_per_pixel];
        for x in 0..width {
            let p = &row[x * bytes_per_pixel..x * bytes_per_pixel + bytes_per_pixel];
            let pixel = match masks {
                Some(masks) => {
                    let word = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                    let channel = |mask: u32| ((word & mask) >> mask.trailing_zeros()) as u8;
                    // A bitmap with masks has no say on transparency here.
                    image::Rgba([channel(masks[0]), channel(masks[1]), channel(masks[2]), 255])
                }
                None => image::Rgba([p[2], p[1], p[0], if has_alpha { p[3] } else { 255 }]),
            };
            img.put_pixel(x as u32, y as u32, pixel);
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x1 bitmap: a header, `masks` if any, then two pixels.
    fn dib(compression: u32, masks: &[u32], pixels: [u32; 2]) -> Vec<u32> {
        let mut out = vec![40, 2, 1, 1 | (32 << 16), compression, 8, 0, 0, 0, 0];
        out.extend_from_slice(masks);
        out.extend_from_slice(&pixels);
        out
    }

    fn decode(words: &[u32]) -> Result<RgbaImage, CmdError> {
        unsafe { decode_dib(words.as_ptr() as *const u8, words.len() * 4) }
    }

    #[test]
    fn bit_masks_say_where_the_colors_are() {
        let bgr = dib(BI_BITFIELDS.0, &[0xff0000, 0xff00, 0xff], [0x00112233, 0x00445566]);
        let img = decode(&bgr).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [0x11, 0x22, 0x33, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0x44, 0x55, 0x66, 255]);

        let rgb = dib(BI_BITFIELDS.0, &[0xff, 0xff00, 0xff0000], [0x00112233, 0x00445566]);
        assert_eq!(decode(&rgb).unwrap().get_pixel(0, 0).0, [0x33, 0x22, 0x11, 255]);
    }

    #[test]
    fn plain_32_bit_pixels_keep_their_alpha() {
        let img = decode(&dib(BI_RGB.0, &[], [0x80112233, 0x00445566])).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [0x11, 0x22, 0x33, 0x80]);
    }

    #[test]
    fn odd_or_cut_off_bitmaps_are_errors_not_garbage() {
        assert!(decode(&dib(BI_BITFIELDS.0, &[0xf800, 0x7e0, 0x1f], [0, 0])).is_err());
        assert!(decode(&dib(1, &[], [0, 0])).is_err());
        let whole = dib(BI_BITFIELDS.0, &[0xff0000, 0xff00, 0xff], [0, 0]);
        assert!(decode(&whole[..whole.len() - 1]).is_err());
    }
}

/// Loads an image file and puts it on the clipboard as a 32-bit CF_DIB.
pub fn set_image(path: &str) -> Result<(), CmdError> {
    let img = image::open(path)
        .map_err(|e| CmdError::invalid_args(format!("could not read image: {e}")))?
        .to_rgba8();
    let (width, height) = img.dimensions();

    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        biHeight: height as i32, // positive = bottom-up, the conventional DIB layout
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: width * height * 4,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            &header as *const _ as *const u8,
            std::mem::size_of::<BITMAPINFOHEADER>(),
        )
    };

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        let dst_row = height - 1 - y; // bottom-up
        for x in 0..width {
            let p = img.get_pixel(x, y).0;
            let i = (dst_row * width + x) as usize * 4;
            pixels[i] = p[2];
            pixels[i + 1] = p[1];
            pixels[i + 2] = p[0];
            pixels[i + 3] = p[3];
        }
    }

    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| CmdError::os(e.to_string()))?;
        let result = (|| -> Result<(), CmdError> {
            EmptyClipboard().map_err(|e| CmdError::os(e.to_string()))?;
            let total = header_bytes.len() + pixels.len();
            let hmem = GlobalAlloc(GMEM_MOVEABLE, total).map_err(|e| CmdError::os(e.to_string()))?;
            let ptr = GlobalLock(hmem) as *mut u8;
            if ptr.is_null() {
                return Err(CmdError::os("clipboard buffer could not be locked"));
            }
            std::ptr::copy_nonoverlapping(header_bytes.as_ptr(), ptr, header_bytes.len());
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr(),
                ptr.add(header_bytes.len()),
                pixels.len(),
            );
            let _ = GlobalUnlock(hmem);
            SetClipboardData(CF_DIB.0 as u32, HANDLE(hmem.0))
                .map_err(|e| CmdError::os(e.to_string()))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

pub fn set_text(text: &str) -> Result<(), CmdError> {
    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| CmdError::os(e.to_string()))?;
        let result = (|| -> Result<(), CmdError> {
            EmptyClipboard().map_err(|e| CmdError::os(e.to_string()))?;
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = wide.len() * std::mem::size_of::<u16>();
            let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| CmdError::os(e.to_string()))?;
            let ptr = GlobalLock(hmem) as *mut u16;
            if ptr.is_null() {
                return Err(CmdError::os("clipboard buffer could not be locked"));
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(hmem);
            SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(hmem.0))
                .map_err(|e| CmdError::os(e.to_string()))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}
