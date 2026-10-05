use crate::output::CmdError;
use image::RgbaImage;
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, DeleteDC, DeleteObject,
    GetDC, GetDIBits, LineTo, MoveToEx, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    TextOutW, BITMAPINFO, BI_RGB, DIB_RGB_COLORS, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, GetSystemMetrics, CURSORINFO, CURSOR_SHOWING,
    DI_NORMAL, ICONINFO, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

struct CaptureRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn virtual_screen_rect() -> CaptureRect {
    unsafe {
        CaptureRect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

fn window_capture_rect(id: &str) -> Result<CaptureRect, CmdError> {
    let hwnd = super::window::parse_id(id)?;
    let (x, y, w, h) = super::window::rect(hwnd)?;
    Ok(CaptureRect { x, y, w, h })
}

fn monitor_capture_rect(id: &str) -> Result<CaptureRect, CmdError> {
    let m = super::monitor::find(id)?;
    Ok(CaptureRect { x: m.x, y: m.y, w: m.width, h: m.height })
}

fn parse_region(spec: &str) -> Result<CaptureRect, CmdError> {
    let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).collect();
    if parts.len() != 4 {
        return Err(CmdError::invalid_args(
            "--region must be formatted as X,Y,W,H",
        ));
    }
    let nums: Result<Vec<i32>, _> = parts.iter().map(|p| p.parse::<i32>()).collect();
    let nums = nums.map_err(|_| CmdError::invalid_args("--region values must be integers"))?;
    Ok(CaptureRect { x: nums[0], y: nums[1], w: nums[2], h: nums[3] })
}

unsafe fn draw_cursor(hdc: windows::Win32::Graphics::Gdi::HDC, origin_x: i32, origin_y: i32) {
    let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    if GetCursorInfo(&mut info).is_err() || info.flags != CURSOR_SHOWING {
        return;
    }
    let mut icon_info = ICONINFO::default();
    if GetIconInfo(info.hCursor, &mut icon_info).is_err() {
        return;
    }
    let x = info.ptScreenPos.x - origin_x - icon_info.xHotspot as i32;
    let y = info.ptScreenPos.y - origin_y - icon_info.yHotspot as i32;
    let _ = DrawIconEx(hdc, x, y, info.hCursor, 0, 0, 0, None, DI_NORMAL);
    if !icon_info.hbmColor.is_invalid() {
        let _ = DeleteObject(icon_info.hbmColor);
    }
    if !icon_info.hbmMask.is_invalid() {
        let _ = DeleteObject(icon_info.hbmMask);
    }
}

/// Overlays a coordinate grid labeled in absolute screen pixels, matching the
/// coordinate space `mouse move`/`mouse click` expect, so an agent can read
/// click targets straight off the screenshot.
unsafe fn draw_grid(hdc: windows::Win32::Graphics::Gdi::HDC, origin_x: i32, origin_y: i32, w: i32, h: i32) {
    const STEP: i32 = 100;
    let color = COLORREF(0x0000FF00); // 0x00BBGGRR -> opaque green
    let pen = CreatePen(PS_SOLID, 1, color);
    let old_pen = SelectObject(hdc, pen);
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, color);

    let mut x = STEP;
    while x < w {
        let _ = MoveToEx(hdc, x, 0, None);
        let _ = LineTo(hdc, x, h);
        x += STEP;
    }
    let mut y = STEP;
    while y < h {
        let _ = MoveToEx(hdc, 0, y, None);
        let _ = LineTo(hdc, w, y);
        y += STEP;
    }

    let mut gy = 0;
    while gy < h {
        let mut gx = 0;
        while gx < w {
            let label = format!("{},{}", origin_x + gx, origin_y + gy);
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = TextOutW(hdc, gx + 2, gy + 2, &wide);
            gx += STEP;
        }
        gy += STEP;
    }

    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

unsafe fn dib_to_image(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    w: i32,
    h: i32,
) -> Result<RgbaImage, CmdError> {
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // negative = top-down, matches our row loop below
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut buffer = vec![0u8; (w as usize) * (h as usize) * 4];
    let copied = GetDIBits(
        hdc,
        bitmap,
        0,
        h as u32,
        Some(buffer.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    if copied == 0 {
        return Err(CmdError::os("GetDIBits failed to copy captured pixels"));
    }

    let mut img = RgbaImage::new(w as u32, h as u32);
    for (i, px) in buffer.chunks_exact(4).enumerate() {
        let x = (i as u32) % w as u32;
        let y = (i as u32) / w as u32;
        img.put_pixel(x, y, image::Rgba([px[2], px[1], px[0], 255]));
    }
    Ok(img)
}

unsafe fn capture(rect: &CaptureRect, cursor: bool, grid: bool) -> Result<RgbaImage, CmdError> {
    let screen_dc = GetDC(HWND::default());
    if screen_dc.is_invalid() {
        return Err(CmdError::os("could not get screen device context"));
    }
    let mem_dc = CreateCompatibleDC(screen_dc);
    let bitmap = CreateCompatibleBitmap(screen_dc, rect.w, rect.h);
    let old_bitmap = SelectObject(mem_dc, bitmap);

    let blt_ok = BitBlt(mem_dc, 0, 0, rect.w, rect.h, screen_dc, rect.x, rect.y, SRCCOPY);

    let result = (|| -> Result<RgbaImage, CmdError> {
        blt_ok.map_err(|e| CmdError::os(e.to_string()))?;
        if cursor {
            draw_cursor(mem_dc, rect.x, rect.y);
        }
        if grid {
            draw_grid(mem_dc, rect.x, rect.y, rect.w, rect.h);
        }
        dib_to_image(mem_dc, bitmap, rect.w, rect.h)
    })();

    SelectObject(mem_dc, old_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(mem_dc);
    ReleaseDC(HWND::default(), screen_dc);

    result
}

/// Captures pixels without PNG-encoding them, for callers (like OCR) that
/// feed the image into another API rather than returning it to the user.
/// Returns the image plus its capture origin in absolute desktop pixels, so
/// callers can translate coordinates found within it back to screen space.
pub(crate) fn capture_raw(
    monitor: Option<&str>,
    window: Option<&str>,
) -> Result<(RgbaImage, i32, i32), CmdError> {
    if monitor.is_some() && window.is_some() {
        return Err(CmdError::invalid_args("specify at most one of --monitor, --window"));
    }

    let rect = if let Some(id) = window {
        window_capture_rect(id)?
    } else if let Some(id) = monitor {
        monitor_capture_rect(id)?
    } else {
        virtual_screen_rect()
    };

    if rect.w <= 0 || rect.h <= 0 {
        return Err(CmdError::invalid_args("capture area has zero size"));
    }

    let img = unsafe { capture(&rect, false, false)? };
    Ok((img, rect.x, rect.y))
}

pub fn take(
    monitor: Option<&str>,
    window: Option<&str>,
    region: Option<&str>,
    scale: Option<f32>,
    cursor: bool,
    grid: bool,
) -> Result<(u32, u32, Vec<u8>), CmdError> {
    let specified = [monitor.is_some(), window.is_some(), region.is_some()]
        .iter()
        .filter(|&&b| b)
        .count();
    if specified > 1 {
        return Err(CmdError::invalid_args(
            "specify at most one of --monitor, --window, --region",
        ));
    }

    let rect = if let Some(id) = window {
        window_capture_rect(id)?
    } else if let Some(id) = monitor {
        monitor_capture_rect(id)?
    } else if let Some(spec) = region {
        parse_region(spec)?
    } else {
        virtual_screen_rect()
    };

    if rect.w <= 0 || rect.h <= 0 {
        return Err(CmdError::invalid_args("capture area has zero size"));
    }

    let mut img = unsafe { capture(&rect, cursor, grid)? };

    if let Some(scale) = scale {
        if scale <= 0.0 {
            return Err(CmdError::invalid_args("--scale must be positive"));
        }
        let new_w = ((img.width() as f32) * scale).round().max(1.0) as u32;
        let new_h = ((img.height() as f32) * scale).round().max(1.0) as u32;
        img = image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Lanczos3);
    }

    let (width, height) = img.dimensions();
    let mut bytes: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| CmdError::os(e.to_string()))?;
    Ok((width, height, bytes))
}
