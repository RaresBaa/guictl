//! `menu`: walks a window's menu bar down to an item through UI Automation,
//! so an agent doesn't have to hover its way through nested popups.

use super::element::automation;
use crate::cli::MouseButton;
use crate::output::CmdError;
use std::time::{Duration, Instant};
use windows::core::VARIANT;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{
    IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationInvokePattern, TreeScope_Descendants, UIA_ControlTypePropertyId,
    UIA_ExpandCollapsePatternId, UIA_InvokePatternId, UIA_MenuItemControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SendMessageTimeoutW, SMTO_ABORTIFHUNG,
    WM_CANCELMODE,
};

/// How long to wait for each submenu, and its item, to appear.
const LEVEL_TIMEOUT: Duration = Duration::from_millis(2000);

/// Lowercase, without the & that marks an access key, a trailing "..." or
/// the shortcut some apps append after a tab.
fn normalize(name: &str) -> String {
    let name = name.split('\t').next().unwrap_or_default().replace('&', "");
    name.trim().trim_end_matches("...").trim_end_matches('…').trim().to_lowercase()
}

/// All visible top-level windows of a process, titled or not: popup menus
/// are untitled top-level windows of their own.
fn process_windows(pid: u32) -> Vec<HWND> {
    extern "system" fn add(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let (pid, out) = unsafe { &mut *(lparam.0 as *mut (u32, Vec<HWND>)) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == *pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            out.push(hwnd);
        }
        BOOL(1)
    }
    let mut state = (pid, Vec::new());
    let _ = unsafe { EnumWindows(Some(add), LPARAM(&mut state as *mut _ as isize)) };
    state.1
}

/// The menu items under `container`, best match first: an exact name before
/// one that only starts with the wanted text.
unsafe fn find_item(
    automation: &IUIAutomation,
    container: &IUIAutomationElement,
    wanted: &str,
) -> Option<IUIAutomationElement> {
    let condition = automation
        .CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_MenuItemControlTypeId.0))
        .ok()?;
    let items = container.FindAll(TreeScope_Descendants, &condition).ok()?;
    let mut prefix_match = None;
    for i in 0..items.Length().unwrap_or(0) {
        let Ok(item) = items.GetElement(i) else { continue };
        let name = normalize(&item.CurrentName().map(|b| b.to_string()).unwrap_or_default());
        if name == wanted {
            return Some(item);
        }
        if prefix_match.is_none() && !wanted.is_empty() && name.starts_with(wanted) {
            prefix_match = Some(item);
        }
    }
    prefix_match
}

unsafe fn click_center(item: &IUIAutomationElement) -> Result<(), CmdError> {
    let r = item.CurrentBoundingRectangle().map_err(|e| CmdError::os(e.to_string()))?;
    super::mouse::click(MouseButton::Left, false, Some(((r.left + r.right) / 2, (r.top + r.bottom) / 2)))
}

/// Opens a submenu: Expand where the app supports it, else a click, which
/// opens a submenu in every toolkit.
unsafe fn open(item: &IUIAutomationElement) -> Result<(), CmdError> {
    if let Ok(pattern) = item.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId) {
        if pattern.Expand().is_ok() {
            return Ok(());
        }
    }
    click_center(item)
}

unsafe fn activate(item: &IUIAutomationElement) -> Result<(), CmdError> {
    if let Ok(pattern) = item.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
        if pattern.Invoke().is_ok() {
            return Ok(());
        }
    }
    click_center(item)
}

pub fn select(hwnd: HWND, path: &[&str]) -> Result<(), CmdError> {
    super::window::check_access(hwnd, "This window")?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    // A menu that opens with a click needs its window in front. Opening
    // through UI Automation doesn't, so carry on if Windows refuses.
    let _ = super::window::focus(hwnd);
    let before = process_windows(pid);
    let result = walk(hwnd, pid, before.clone(), path);
    if result.is_err() {
        close_menus(hwnd, pid, &before);
    }
    result
}

/// Closes whatever menus a failed walk left open. WM_CANCELMODE makes a
/// standard Windows menu close entirely; other toolkits get an Escape per
/// popup that is still showing.
fn close_menus(hwnd: HWND, pid: u32, before: &[HWND]) {
    unsafe {
        SendMessageTimeoutW(hwnd, WM_CANCELMODE, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 500, None);
    }
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(100));
        if process_windows(pid).iter().all(|w| before.contains(w)) {
            return;
        }
        let _ = super::keyboard::key("escape", 1);
    }
}

fn walk(hwnd: HWND, pid: u32, mut known: Vec<HWND>, path: &[&str]) -> Result<(), CmdError> {
    let automation = automation()?;
    let root = unsafe { automation.ElementFromHandle(hwnd) }.map_err(|e| CmdError::os(e.to_string()))?;
    for (depth, name) in path.iter().enumerate() {
        let wanted = normalize(name);
        let start = Instant::now();
        let item = loop {
            // A submenu usually opens as a new popup window of the app;
            // search those first, newest first, so an item that also exists
            // in an outer menu isn't picked from there. Some toolkits draw
            // their menus inside the window instead.
            let popups: Vec<HWND> = process_windows(pid).into_iter().filter(|w| !known.contains(w)).collect();
            let found = popups
                .iter()
                .filter_map(|&w| unsafe { automation.ElementFromHandle(w) }.ok())
                .chain(std::iter::once(root.clone()))
                .find_map(|container| unsafe { find_item(&automation, &container, &wanted) });
            if let Some(item) = found {
                known.extend(popups);
                break item;
            }
            if start.elapsed() >= LEVEL_TIMEOUT {
                let at = path[..depth].join(">");
                let place = if at.is_empty() { "the menu bar".to_string() } else { format!("the menu {at}") };
                return Err(CmdError::not_found(format!("no item \"{name}\" in {place}")));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if depth + 1 == path.len() {
            unsafe { activate(&item) }?;
        } else {
            unsafe { open(&item) }?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn names_compare_without_decoration() {
        assert_eq!(normalize("Save &As..."), "save as");
        assert_eq!(normalize("&Zoom In\tCtrl+Plus"), "zoom in");
        assert_eq!(normalize(" Print… "), "print");
    }
}
