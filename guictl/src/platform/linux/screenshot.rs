use super::{dbus, gnome, kwin};
use crate::output::CmdError;
use crate::platform::Screenshot;
use image::{Rgba, RgbaImage};
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::OwnedFd;
use zbus::zvariant::{DynamicType, Fd, OwnedValue, Value};

const PATH: &str = "/org/kde/KWin/ScreenShot2";
pub(super) const INTERFACE: &str = "org.kde.KWin.ScreenShot2";

struct CaptureRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

enum Target<'a> {
    Workspace,
    Monitor(&'a str),
    Window(&'a str),
    Region(CaptureRect),
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

fn not_authorized() -> CmdError {
    CmdError::permission_denied(
        "KWin has not been told to let this executable take screenshots. Run `guictl setup` once (and again if the binary moves)",
    )
}

fn call<B: serde::Serialize + DynamicType>(
    conn: &zbus::blocking::Connection,
    method: &str,
    body: &B,
) -> Result<HashMap<String, OwnedValue>, CmdError> {
    conn.call_method(Some(kwin::SERVICE), PATH, Some(INTERFACE), method, body)
        .and_then(|reply| reply.body().deserialize())
        .map_err(|e| {
            let message = e.to_string();
            if message.contains("NoAuthorized") {
                not_authorized()
            } else if message.contains("ServiceUnknown") || message.contains("UnknownObject") {
                CmdError::unsupported(format!(
                    "KWin's screenshot interface is not available (is this a KDE Plasma session?): {message}"
                ))
            } else if message.contains("InvalidWindow") {
                CmdError::not_found("no window with that id")
            } else {
                CmdError::os(format!("KWin could not take the screenshot: {message}"))
            }
        })
}

fn meta_u32(meta: &HashMap<String, OwnedValue>, key: &str) -> Result<u32, CmdError> {
    meta.get(key)
        .and_then(|v| u32::try_from(&**v).ok())
        .ok_or_else(|| CmdError::os(format!("KWin's screenshot reply has no usable '{key}'")))
}

/// Captures through guictl's GNOME Shell extension, which writes a PNG for
/// guictl to read. The shell captures at the screen's real resolution, so on a
/// scaled display the image is brought down to logical pixels, the coordinates
/// every other command works in.
fn capture_gnome(target: &Target, cursor: bool) -> Result<RgbaImage, CmdError> {
    let file = gnome::TempFile::new("screenshot.png");
    let (args, logical) = match target {
        Target::Workspace => {
            let monitors = super::monitor::list()?;
            let attached = || monitors.iter().filter(|m| m.attached);
            let left = attached().map(|m| m.x).min().unwrap_or(0);
            let top = attached().map(|m| m.y).min().unwrap_or(0);
            let right = attached().map(|m| m.x + m.width).max().unwrap_or(0);
            let bottom = attached().map(|m| m.y + m.height).max().unwrap_or(0);
            (serde_json::json!({ "target": "workspace", "cursor": cursor }), (right - left, bottom - top))
        }
        Target::Monitor(id) => {
            let m = super::monitor::find(id)?;
            (
                serde_json::json!({ "target": "area", "x": m.x, "y": m.y, "w": m.width, "h": m.height }),
                (m.width, m.height),
            )
        }
        Target::Window(id) => {
            let (_, _, w, h) = super::window::rect(super::window::parse_id(id)?)?;
            (serde_json::json!({ "target": "window", "id": id }), (w, h))
        }
        Target::Region(r) => (
            serde_json::json!({ "target": "area", "x": r.x, "y": r.y, "w": r.w, "h": r.h }),
            (r.w, r.h),
        ),
    };
    let mut args = args;
    args["path"] = serde_json::Value::from(file.path());
    gnome::call("screenshot", args)?;
    let img = image::open(&file.0)
        .map_err(|e| CmdError::os(format!("could not read the screenshot GNOME Shell took: {e}")))?
        .into_rgba8();
    let (w, h) = (logical.0.max(1) as u32, logical.1.max(1) as u32);
    if img.dimensions() == (w, h) || logical.0 <= 0 || logical.1 <= 0 {
        return Ok(img);
    }
    Ok(image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle))
}

/// Captures through KWin's own `ScreenShot2` interface: it hands back raw
/// pixels over a pipe with no interactive picker (unlike the XDG screenshot
/// portal), and can capture a window's contents even when it's covered.
fn capture(target: &Target, cursor: bool) -> Result<RgbaImage, CmdError> {
    if gnome::is_gnome() {
        return capture_gnome(target, cursor);
    }
    let conn = dbus::session()?;
    let (mut reader, writer) =
        std::io::pipe().map_err(|e| CmdError::os(format!("could not create a pipe: {e}")))?;
    let pipe = Fd::from(OwnedFd::from(writer));

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("include-cursor", Value::from(cursor));
    // Not "native-resolution": on a scaled display that would return more
    // pixels than the logical coordinates every other command works in.
    let meta = match target {
        Target::Workspace => call(&conn, "CaptureWorkspace", &(options, pipe))?,
        Target::Monitor(name) => call(&conn, "CaptureScreen", &(*name, options, pipe))?,
        Target::Window(id) => {
            options.insert("include-decoration", Value::from(true));
            // Without the shadow the image matches the window's frame
            // geometry, so image coordinates map straight onto the window.
            options.insert("include-shadow", Value::from(false));
            call(&conn, "CaptureWindow", &(*id, options, pipe))?
        }
        Target::Region(r) => {
            call(&conn, "CaptureArea", &(r.x, r.y, r.w as u32, r.h as u32, options, pipe))?
        }
    };

    let width = meta_u32(&meta, "width")?;
    let height = meta_u32(&meta, "height")?;
    let stride = meta_u32(&meta, "stride")? as usize;
    let format = meta_u32(&meta, "format")?;
    if width == 0 || height == 0 || stride < width as usize * 4 {
        return Err(CmdError::os("KWin returned an empty screenshot"));
    }

    // Read exactly the announced size rather than to EOF, so this doesn't
    // depend on when every copy of the pipe's write end gets closed.
    let mut data = vec![0u8; stride * height as usize];
    reader
        .read_exact(&mut data)
        .map_err(|e| CmdError::os(format!("could not read screenshot data from KWin: {e}")))?;

    // `format` is a QImage::Format. The 32-bit ARGB ones are native-endian
    // words, so B,G,R,A in memory; the RGBA8888 ones are already in byte order.
    let (bgr, premultiplied, has_alpha) = match format {
        4 => (true, false, false),
        5 => (true, false, true),
        6 => (true, true, true),
        16 => (false, false, false),
        17 => (false, false, true),
        18 => (false, true, true),
        other => {
            return Err(CmdError::os(format!(
                "KWin returned a screenshot in an unsupported pixel format ({other})"
            )))
        }
    };
    if bgr && cfg!(target_endian = "big") {
        return Err(CmdError::unsupported("screenshots are not supported on big-endian machines"));
    }

    let mut img = RgbaImage::new(width, height);
    for (y, row) in data.chunks_exact(stride).enumerate() {
        for (x, px) in row[..width as usize * 4].as_chunks::<4>().0.iter().enumerate() {
            let (mut r, mut g, mut b) = if bgr { (px[2], px[1], px[0]) } else { (px[0], px[1], px[2]) };
            let a = if has_alpha { px[3] } else { 255 };
            if premultiplied && a != 0 && a != 255 {
                let unmul = |c: u8| ((c as u32 * 255) / a as u32).min(255) as u8;
                r = unmul(r);
                g = unmul(g);
                b = unmul(b);
            }
            img.put_pixel(x as u32, y as u32, Rgba([r, g, b, a]));
        }
    }
    Ok(img)
}

// 3x5 glyphs for grid labels, one row per element, most significant bit on the left.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        ',' => [0b000, 0b000, 0b000, 0b010, 0b100],
        _ => [0; 5],
    }
}

fn fill(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, color: Rgba<u8>) {
    for py in y.max(0)..(y + h).min(img.height() as i32) {
        for px in x.max(0)..(x + w).min(img.width() as i32) {
            img.put_pixel(px as u32, py as u32, color);
        }
    }
}

fn draw_label(img: &mut RgbaImage, x: i32, y: i32, text: &str) {
    const SCALE: i32 = 2;
    let green = Rgba([0, 255, 0, 255]);
    // A dark backing box keeps the label readable over light content.
    fill(img, x - 1, y - 1, text.len() as i32 * 4 * SCALE + 1, 5 * SCALE + 2, Rgba([0, 0, 0, 255]));
    for (i, c) in text.chars().enumerate() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    let gx = x + (i as i32 * 4 + col) * SCALE;
                    fill(img, gx, y + row as i32 * SCALE, SCALE, SCALE, green);
                }
            }
        }
    }
}

/// Overlays a coordinate grid labeled in absolute screen pixels, matching the
/// coordinate space `mouse move`/`mouse click` expect, so an agent can read
/// click targets straight off the screenshot.
fn draw_grid(img: &mut RgbaImage, origin_x: i32, origin_y: i32) {
    const STEP: i32 = 100;
    let (w, h) = (img.width() as i32, img.height() as i32);
    let green = Rgba([0, 255, 0, 255]);
    for x in (STEP..w).step_by(STEP as usize) {
        fill(img, x, 0, 1, h, green);
    }
    for y in (STEP..h).step_by(STEP as usize) {
        fill(img, 0, y, w, 1, green);
    }
    for gy in (0..h).step_by(STEP as usize) {
        for gx in (0..w).step_by(STEP as usize) {
            draw_label(img, gx + 3, gy + 3, &format!("{},{}", origin_x + gx, origin_y + gy));
        }
    }
}

/// Puts images together on a canvas just big enough for them, in the order
/// given, and returns it with its top-left corner in desktop pixels.
fn compose(mut layers: Vec<(RgbaImage, i32, i32)>) -> (RgbaImage, i32, i32) {
    if layers.len() == 1 {
        return layers.remove(0);
    }
    let left = layers.iter().map(|l| l.1).min().unwrap_or(0);
    let top = layers.iter().map(|l| l.2).min().unwrap_or(0);
    let right = layers.iter().map(|l| l.1 + l.0.width() as i32).max().unwrap_or(0);
    let bottom = layers.iter().map(|l| l.2 + l.0.height() as i32).max().unwrap_or(0);
    let mut canvas = RgbaImage::new((right - left) as u32, (bottom - top) as u32);
    for (image, x, y) in &layers {
        image::imageops::overlay(&mut canvas, image, (x - left) as i64, (y - top) as i64);
    }
    (canvas, left, top)
}

/// A window with its open menus and dropdowns on it. They can reach past the
/// window, and then the image grows to show them whole.
fn capture_window(id: &str, cursor: bool) -> Result<(RgbaImage, i32, i32), CmdError> {
    let window = super::window::with_popups(super::window::parse_id(id)?)?;
    let mut layers = vec![(capture(&Target::Window(id), cursor)?, window.x, window.y)];
    for popup in window.popups {
        match capture(&Target::Window(&popup.id), cursor) {
            Ok(image) => layers.push((image, popup.x, popup.y)),
            // It closed in the meantime, so it isn't on screen any more.
            Err(e) if e.code == "not_found" => {}
            Err(e) => return Err(e),
        }
    }
    Ok(compose(layers))
}

/// Top-left corner of the whole desktop, which isn't 0,0 when a monitor sits
/// left of or above the primary one.
fn workspace_origin() -> Result<(i32, i32), CmdError> {
    let monitors = super::monitor::list()?;
    let attached = || monitors.iter().filter(|m| m.attached);
    Ok((
        attached().map(|m| m.x).min().unwrap_or(0),
        attached().map(|m| m.y).min().unwrap_or(0),
    ))
}

/// Captures pixels without PNG-encoding them, for callers (like OCR) that
/// feed the image into something else rather than returning it to the user.
/// Returns the image plus its capture origin in absolute desktop pixels, so
/// callers can translate coordinates found within it back to screen space.
pub(crate) fn capture_raw(
    monitor: Option<&str>,
    window: Option<&str>,
    region: Option<&str>,
) -> Result<(RgbaImage, i32, i32), CmdError> {
    let (target, (x, y)) = match (monitor, window, region) {
        (None, Some(id), None) => return capture_window(id, false),
        (Some(id), None, None) => {
            let m = super::monitor::find(id)?;
            (Target::Monitor(id), (m.x, m.y))
        }
        (None, None, Some(spec)) => {
            let rect = parse_region(spec)?;
            if rect.w <= 0 || rect.h <= 0 {
                return Err(CmdError::invalid_args("capture area has zero size"));
            }
            let origin = (rect.x, rect.y);
            (Target::Region(rect), origin)
        }
        (None, None, None) => (Target::Workspace, workspace_origin()?),
        _ => return Err(CmdError::invalid_args("specify at most one of --monitor, --window, --region")),
    };
    Ok((capture(&target, false)?, x, y))
}

/// Width, height and PNG bytes, plus for `window` where the image's top-left
/// corner is on the desktop: an open popup can move it off the window's corner.
pub fn take(
    monitor: Option<&str>,
    window: Option<&str>,
    region: Option<&str>,
    scale: Option<f32>,
    cursor: bool,
    grid: bool,
) -> Result<Screenshot, CmdError> {
    let specified = [monitor.is_some(), window.is_some(), region.is_some()]
        .iter()
        .filter(|&&b| b)
        .count();
    if specified > 1 {
        return Err(CmdError::invalid_args(
            "specify at most one of --monitor, --window, --region",
        ));
    }

    let (mut img, origin) = if let Some(id) = window {
        let (img, x, y) = capture_window(id, cursor)?;
        (img, (x, y))
    } else if let Some(id) = monitor {
        let m = super::monitor::find(id)?;
        (capture(&Target::Monitor(id), cursor)?, (m.x, m.y))
    } else if let Some(spec) = region {
        let rect = parse_region(spec)?;
        if rect.w <= 0 || rect.h <= 0 {
            return Err(CmdError::invalid_args("capture area has zero size"));
        }
        let origin = (rect.x, rect.y);
        (capture(&Target::Region(rect), cursor)?, origin)
    } else {
        let origin = if grid { workspace_origin()? } else { (0, 0) };
        (capture(&Target::Workspace, cursor)?, origin)
    };

    if grid {
        draw_grid(&mut img, origin.0, origin.1);
    }

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
        .map_err(|e| CmdError::os(format!("could not encode the screenshot: {e}")))?;
    Ok((width, height, bytes, window.map(|_| origin)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_regions() {
        let r = parse_region("10, 20,300,400").unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (10, 20, 300, 400));
        let r = parse_region("-1920,0,800,600").unwrap();
        assert_eq!(r.x, -1920);
    }

    #[test]
    fn a_window_without_popups_stays_as_it_is() {
        let (image, x, y) = compose(vec![(RgbaImage::new(30, 20), 100, 200)]);
        assert_eq!((image.dimensions(), x, y), ((30, 20), 100, 200));
    }

    #[test]
    fn popups_land_where_they_are_on_the_desktop_and_grow_the_image() {
        let red = Rgba([255, 0, 0, 255]);
        let blue = Rgba([0, 0, 255, 255]);
        let (image, x, y) = compose(vec![
            (RgbaImage::from_pixel(10, 10, red), 100, 100),
            (RgbaImage::from_pixel(4, 4, blue), 108, 95),
        ]);
        assert_eq!((image.dimensions(), x, y), ((12, 15), 100, 95));
        assert_eq!(*image.get_pixel(0, 5), red);
        assert_eq!(*image.get_pixel(8, 0), blue);
        assert_eq!(*image.get_pixel(11, 3), blue);
        assert_eq!(*image.get_pixel(0, 0), Rgba([0, 0, 0, 0]));
        assert_eq!(*image.get_pixel(11, 14), Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn rejects_malformed_regions() {
        for spec in ["", "1,2,3", "1,2,3,4,5", "a,b,c,d", "1.5,2,3,4"] {
            assert_eq!(parse_region(spec).err().unwrap().code, "invalid_args", "{spec}");
        }
    }
}
