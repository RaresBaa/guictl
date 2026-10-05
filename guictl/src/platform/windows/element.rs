use crate::cli::MouseButton;
use crate::output::CmdError;
use serde::Serialize;
use windows::core::BSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationTextPattern, IUIAutomationTreeWalker, IUIAutomationValuePattern,
    UIA_InvokePatternId, UIA_TextPatternId, UIA_ValuePatternId,
};

/// Depth/count caps keep `list`/`find` bounded on apps with huge UI trees
/// (e.g. a browser DOM exposed through UIA), since this is a one-shot CLI
/// call with no way to stream or cancel partway through.
const MAX_DEPTH: usize = 20;
const MAX_RESULTS: usize = 500;

#[derive(Serialize)]
pub struct ElementInfo {
    pub id: String,
    pub name: String,
    pub role: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

fn automation() -> Result<IUIAutomation, CmdError> {
    unsafe {
        // Ignore the result: in this one-shot process COM is uninitialized,
        // so S_OK is expected; a prior init on this thread (S_FALSE) is fine too.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| CmdError::os(format!("could not start UI Automation: {e}")))
    }
}

unsafe fn element_name(el: &IUIAutomationElement) -> String {
    el.CurrentName().map(|b| b.to_string()).unwrap_or_default()
}

unsafe fn element_role(el: &IUIAutomationElement) -> String {
    el.CurrentLocalizedControlType().map(|b| b.to_string()).unwrap_or_default()
}

unsafe fn element_rect(el: &IUIAutomationElement) -> (i32, i32, i32, i32) {
    match el.CurrentBoundingRectangle() {
        Ok(r) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
        Err(_) => (0, 0, 0, 0),
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn collect(
    walker: &IUIAutomationTreeWalker,
    parent: &IUIAutomationElement,
    path: &mut Vec<usize>,
    depth: usize,
    hwnd_id: &str,
    role_filter: Option<&str>,
    name_filter: Option<&str>,
    stop_at_first: bool,
    out: &mut Vec<ElementInfo>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut next = walker.GetFirstChildElement(parent);
    let mut index = 0usize;
    while let Ok(child) = next {
        if out.len() >= MAX_RESULTS || (stop_at_first && !out.is_empty()) {
            return;
        }
        path.push(index);
        let role = element_role(&child);
        let name = element_name(&child);
        let role_ok = role_filter.map_or(true, |r| role.eq_ignore_ascii_case(r));
        let name_ok = name_filter.map_or(true, |n| name.to_lowercase().contains(&n.to_lowercase()));
        if role_ok && name_ok {
            let (x, y, w, h) = element_rect(&child);
            let id = format!(
                "{hwnd_id}/{}",
                path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")
            );
            out.push(ElementInfo { id, name, role, x, y, w, h });
        }
        if !(stop_at_first && !out.is_empty()) {
            collect(walker, &child, path, depth + 1, hwnd_id, role_filter, name_filter, stop_at_first, out);
        }
        path.pop();
        next = walker.GetNextSiblingElement(&child);
        index += 1;
    }
}

fn resolve(id: &str) -> Result<IUIAutomationElement, CmdError> {
    let mut parts = id.split('/');
    let hwnd_part = parts
        .next()
        .ok_or_else(|| CmdError::invalid_args("invalid element id"))?;
    let hwnd = super::window::parse_id(hwnd_part)?;
    let automation = automation()?;
    unsafe {
        let mut el = automation
            .ElementFromHandle(hwnd)
            .map_err(|e| CmdError::os(e.to_string()))?;
        let walker = automation
            .ControlViewWalker()
            .map_err(|e| CmdError::os(e.to_string()))?;
        for part in parts {
            let target_index: usize = part
                .parse()
                .map_err(|_| CmdError::invalid_args("invalid element id"))?;
            let not_found = || CmdError::not_found("element no longer exists; re-run element list/find");
            let mut child = walker.GetFirstChildElement(&el).map_err(|_| not_found())?;
            for _ in 0..target_index {
                child = walker.GetNextSiblingElement(&child).map_err(|_| not_found())?;
            }
            el = child;
        }
        Ok(el)
    }
}

pub fn list(hwnd: HWND, role: Option<&str>) -> Result<Vec<ElementInfo>, CmdError> {
    let hwnd_id = format!("{:#x}", hwnd.0 as usize);
    let automation = automation()?;
    unsafe {
        let root = automation
            .ElementFromHandle(hwnd)
            .map_err(|e| CmdError::os(e.to_string()))?;
        let walker = automation
            .ControlViewWalker()
            .map_err(|e| CmdError::os(e.to_string()))?;
        let mut out = Vec::new();
        let mut path = Vec::new();
        collect(&walker, &root, &mut path, 0, &hwnd_id, role, None, false, &mut out);
        Ok(out)
    }
}

pub fn find(hwnd: HWND, name: Option<&str>, role: Option<&str>) -> Result<ElementInfo, CmdError> {
    let hwnd_id = format!("{:#x}", hwnd.0 as usize);
    let automation = automation()?;
    unsafe {
        let root = automation
            .ElementFromHandle(hwnd)
            .map_err(|e| CmdError::os(e.to_string()))?;
        let walker = automation
            .ControlViewWalker()
            .map_err(|e| CmdError::os(e.to_string()))?;
        let mut out = Vec::new();
        let mut path = Vec::new();
        collect(&walker, &root, &mut path, 0, &hwnd_id, role, name, true, &mut out);
        out.into_iter().next().ok_or_else(|| CmdError::not_found("no matching element"))
    }
}

pub fn click(id: &str) -> Result<(), CmdError> {
    let el = resolve(id)?;
    let (x, y, w, h) = unsafe { element_rect(&el) };
    super::mouse::click(MouseButton::Left, false, Some((x + w / 2, y + h / 2)))
}

/// Prefers the UIA Invoke pattern (press the control without moving the
/// mouse); falls back to a synthesized click at the element's center for
/// controls that don't expose it.
pub fn invoke(id: &str) -> Result<(), CmdError> {
    let el = resolve(id)?;
    unsafe {
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
            return pattern.Invoke().map_err(|e| CmdError::os(e.to_string()));
        }
        let (x, y, w, h) = element_rect(&el);
        super::mouse::click(MouseButton::Left, false, Some((x + w / 2, y + h / 2)))
    }
}

pub fn text_get(id: &str) -> Result<String, CmdError> {
    let el = resolve(id)?;
    unsafe {
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            return pattern.CurrentValue().map(|b| b.to_string()).map_err(|e| CmdError::os(e.to_string()));
        }
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
            let range = pattern.DocumentRange().map_err(|e| CmdError::os(e.to_string()))?;
            return range.GetText(-1).map(|b| b.to_string()).map_err(|e| CmdError::os(e.to_string()));
        }
    }
    Err(CmdError::unsupported(
        "element does not support reading text (no Value or Text pattern)",
    ))
}

/// Prefers the UIA Value pattern; falls back to focusing the element and
/// synthesizing select-all + type, for custom-drawn editors that don't
/// expose ValuePattern.
pub fn text_set(id: &str, text: &str) -> Result<(), CmdError> {
    let el = resolve(id)?;
    unsafe {
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            let value = BSTR::from(text);
            return pattern.SetValue(&value).map_err(|e| CmdError::os(e.to_string()));
        }
        let _ = el.SetFocus();
    }
    super::keyboard::hotkey("ctrl+a")?;
    super::keyboard::type_text(text)
}
