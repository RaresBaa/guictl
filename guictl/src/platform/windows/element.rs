use crate::cli::MouseButton;
use crate::output::CmdError;
use serde::Serialize;
use windows::core::{BSTR, VARIANT};
use windows::Win32::Foundation::{E_INVALIDARG, HWND, POINT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, ExpandCollapseState_Collapsed, ExpandCollapseState_Expanded, ExpandCollapseState_LeafNode,
    IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationRangeValuePattern, IUIAutomationSelectionItemPattern,
    IUIAutomationTextPattern, IUIAutomationTogglePattern, IUIAutomationTreeWalker, IUIAutomationValuePattern,
    ToggleState_Off, ToggleState_On, TreeScope_Descendants, UIA_ExpandCollapsePatternId,
    UIA_IsSelectionItemPatternAvailablePropertyId, UIA_RangeValuePatternId, ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_RADIOBUTTON, UIA_ButtonControlTypeId,
    UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_CustomControlTypeId,
    UIA_DataItemControlTypeId, UIA_EditControlTypeId, UIA_InvokePatternId,
    UIA_LegacyIAccessiblePatternId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId,
    UIA_RadioButtonControlTypeId, UIA_SelectionItemPatternId, UIA_SliderControlTypeId,
    UIA_SpinnerControlTypeId, UIA_TabItemControlTypeId, UIA_TextPatternId, UIA_TogglePatternId,
    UIA_TreeItemControlTypeId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, WindowFromPoint, GA_ROOT, STATE_SYSTEM_CHECKED, STATE_SYSTEM_MIXED,
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
    /// The current contents of a field, combo box, spinner or slider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Whether a check box, radio button, tab, toggle, or checkable list, tree
    /// or menu item is on; absent for controls that can't be checked or don't
    /// report it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Set on a half-checked box, which has no "checked".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mixed: bool,
    /// Set on a control that is greyed out and ignores input for now.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

pub(super) fn automation() -> Result<IUIAutomation, CmdError> {
    unsafe {
        // Ignore the result: in this one-shot process COM is uninitialized,
        // so S_OK is expected; a prior init on this thread (S_FALSE) is fine too.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| CmdError::os(format!("could not start UI Automation: {e}")))
    }
}

unsafe fn element_name(el: &IUIAutomationElement) -> String {
    unsafe {
        el.CurrentName().map(|b| b.to_string()).unwrap_or_default()
    }
}

unsafe fn element_role(el: &IUIAutomationElement) -> String {
    unsafe {
        el.CurrentLocalizedControlType().map(|b| b.to_string()).unwrap_or_default()
    }
}

/// Values longer than this are cut short in listings; `element text get`
/// reads the whole thing.
const MAX_VALUE_CHARS: usize = 200;

/// The state worth showing next to an element in a listing. Only asked of
/// the control types that have it: each question is a round trip to the
/// app, and a listing can hold hundreds of elements.
unsafe fn element_state(el: &IUIAutomationElement) -> (Option<String>, Option<bool>, bool) {
    unsafe {
        let Ok(control) = el.CurrentControlType() else { return (None, None, false) };
        if [UIA_EditControlTypeId, UIA_ComboBoxControlTypeId, UIA_SpinnerControlTypeId, UIA_SliderControlTypeId]
            .contains(&control)
        {
            let value = el
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|p| p.CurrentValue())
                .ok()
                .map(|b| {
                    let text = b.to_string();
                    match text.char_indices().nth(MAX_VALUE_CHARS) {
                        Some((cut, _)) => format!("{}…", &text[..cut]),
                        None => text,
                    }
                })
                .or_else(|| {
                    // A slider or spinner may only report a number.
                    el.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
                        .and_then(|p| p.CurrentValue())
                        .ok()
                        .map(|v| v.to_string())
                });
            return (value, None, false);
        }
        if [
            UIA_CheckBoxControlTypeId,
            UIA_ButtonControlTypeId,
            UIA_MenuItemControlTypeId,
            UIA_ListItemControlTypeId,
            UIA_TreeItemControlTypeId,
            UIA_DataItemControlTypeId,
            UIA_CustomControlTypeId,
        ]
        .contains(&control)
        {
            let state = el
                .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                .and_then(|p| p.CurrentToggleState());
            let (checked, mixed) = match state {
                Ok(s) if s == ToggleState_On => (Some(true), false),
                Ok(s) if s == ToggleState_Off => (Some(false), false),
                Ok(_) => (None, true),
                Err(_) => legacy_checked(el),
            };
            return (None, checked, mixed);
        }
        if [UIA_RadioButtonControlTypeId, UIA_TabItemControlTypeId].contains(&control) {
            let selected = el
                .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                .and_then(|p| p.CurrentIsSelected())
                .ok()
                .map(|b| b.as_bool());
            return (None, selected, false);
        }
        (None, None, false)
    }
}

/// The check mark as MSAA reports it, for controls with no Toggle pattern,
/// such as the items of a native menu. MSAA only flags "checked", so a
/// missing flag means "off" only for a role that is always checkable.
unsafe fn legacy_checked(el: &IUIAutomationElement) -> (Option<bool>, bool) {
    unsafe {
        let Ok(legacy) = el.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
        else {
            return (None, false);
        };
        let Ok(state) = legacy.CurrentState() else { return (None, false) };
        if state & STATE_SYSTEM_MIXED != 0 {
            return (None, true);
        }
        if state & STATE_SYSTEM_CHECKED != 0 {
            return (Some(true), false);
        }
        let Ok(role) = legacy.CurrentRole() else { return (None, false) };
        ([ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_RADIOBUTTON].contains(&role).then_some(false), false)
    }
}

unsafe fn element_rect(el: &IUIAutomationElement) -> (i32, i32, i32, i32) {
    unsafe {
        match el.CurrentBoundingRectangle() {
            Ok(r) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
            Err(_) => (0, 0, 0, 0),
        }
    }
}

unsafe fn describe(el: &IUIAutomationElement, id: String) -> ElementInfo {
    unsafe {
        let (x, y, w, h) = element_rect(el);
        let (value, checked, mixed) = element_state(el);
        let disabled = el.CurrentIsEnabled().is_ok_and(|enabled| !enabled.as_bool());
        ElementInfo { id, name: element_name(el), role: element_role(el), x, y, w, h, value, checked, mixed, disabled }
    }
}

#[allow(clippy::too_many_arguments)]
fn is_exact(el: &ElementInfo, name: Option<&str>) -> bool {
    name.is_none_or(|n| el.name.to_lowercase() == n.to_lowercase())
}

fn has_exact(found: &[ElementInfo], name: Option<&str>) -> bool {
    found.iter().any(|el| is_exact(el, name))
}

/// What `collect` looks for, the same at every level of the tree.
struct Search<'a> {
    walker: &'a IUIAutomationTreeWalker,
    hwnd_id: &'a str,
    role_filter: Option<&'a str>,
    name_filter: Option<&'a str>,
    stop_at_first: bool,
}

unsafe fn collect(
    search: &Search,
    parent: &IUIAutomationElement,
    path: &mut Vec<usize>,
    depth: usize,
    out: &mut Vec<ElementInfo>,
) {
    let Search { walker, hwnd_id, role_filter, name_filter, stop_at_first } = *search;
    unsafe {
        if depth > MAX_DEPTH {
            return;
        }
        let mut next = walker.GetFirstChildElement(parent);
        let mut index = 0usize;
        while let Ok(child) = next {
            if out.len() >= MAX_RESULTS || (stop_at_first && has_exact(out, name_filter)) {
                return;
            }
            path.push(index);
            let role = element_role(&child);
            let name = element_name(&child);
            let role_ok = role_filter.is_none_or(|r| role.eq_ignore_ascii_case(r));
            let name_ok = name_filter.is_none_or(|n| name.to_lowercase().contains(&n.to_lowercase()));
            if role_ok && name_ok {
                let id = format!(
                    "{hwnd_id}/{}",
                    path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("/")
                );
                out.push(describe(&child, id));
            }
            if !(stop_at_first && has_exact(out, name_filter)) {
                collect(search, &child, path, depth + 1, out);
            }
            path.pop();
            next = walker.GetNextSiblingElement(&child);
            index += 1;
        }
    }
}

fn resolve(id: &str) -> Result<IUIAutomationElement, CmdError> {
    let mut parts = id.split('/');
    let hwnd_part = parts
        .next()
        .ok_or_else(|| CmdError::invalid_args("invalid element id"))?;
    let hwnd = super::window::parse_id(hwnd_part)?;
    super::window::check_access(hwnd, "This element's window")?;
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

fn search(
    hwnd: HWND,
    role: Option<&str>,
    name: Option<&str>,
    stop_at_first: bool,
) -> Result<Vec<ElementInfo>, CmdError> {
    // The tree of such a window reads as just its title bar.
    super::window::check_access(hwnd, "This window")?;
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
        let search = Search { walker: &walker, hwnd_id: &hwnd_id, role_filter: role, name_filter: name, stop_at_first };
        collect(&search, &root, &mut path, 0, &mut out);
        Ok(out)
    }
}

pub fn list(hwnd: HWND, role: Option<&str>) -> Result<Vec<ElementInfo>, CmdError> {
    search(hwnd, role, None, false)
}

pub fn find(hwnd: HWND, name: Option<&str>, role: Option<&str>) -> Result<ElementInfo, CmdError> {
    let out = search(hwnd, role, name, true)?;
    // An exact name wins over one that merely contains the text:
    // "Discard" should find the Discard button, not a label above it
    // asking whether to discard.
    let index = out.iter().position(|el| is_exact(el, name)).unwrap_or(0);
    out.into_iter().nth(index).ok_or_else(|| CmdError::not_found("no matching element"))
}

/// Every match, in the order `find` prefers them.
pub fn find_all(hwnd: HWND, name: Option<&str>, role: Option<&str>) -> Result<Vec<ElementInfo>, CmdError> {
    let mut out = search(hwnd, role, name, false)?;
    out.sort_by_key(|el| !is_exact(el, name));
    Ok(out)
}

pub fn rect(id: &str) -> Result<(i32, i32, i32, i32), CmdError> {
    Ok(unsafe { element_rect(&resolve(id)?) })
}

/// The element Windows finds at a point. Its id is the same path through the
/// tree that `list` prints, worked out by climbing to the window.
pub fn at(x: i32, y: i32) -> Result<ElementInfo, CmdError> {
    let point = POINT { x, y };
    let hwnd = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
    if hwnd.0.is_null() {
        return Err(CmdError::not_found(format!("no window at {x},{y}")));
    }
    super::window::check_access(hwnd, "The window at that point")?;
    let hwnd_id = format!("{:#x}", hwnd.0 as usize);
    let automation = automation()?;
    let outside = || CmdError::not_found(format!("no element of the window at {x},{y} is there"));
    unsafe {
        let os_err = |e: windows::core::Error| CmdError::os(e.to_string());
        let root = automation.ElementFromHandle(hwnd).map_err(os_err)?;
        let walker = automation.ControlViewWalker().map_err(os_err)?;
        let found = automation.ElementFromPoint(point).map_err(os_err)?;
        let found = walker.NormalizeElement(&found).map_err(|_| outside())?;
        let mut path = Vec::new();
        let mut el = found.clone();
        while !automation.CompareElements(&el, &root).map_err(os_err)?.as_bool() {
            let mut index = 0usize;
            let mut sibling = el.clone();
            while let Ok(previous) = walker.GetPreviousSiblingElement(&sibling) {
                sibling = previous;
                index += 1;
            }
            path.push(index.to_string());
            el = walker.GetParentElement(&el).map_err(|_| outside())?;
        }
        path.push(hwnd_id);
        path.reverse();
        Ok(describe(&found, path.join("/")))
    }
}

/// A real click at the element's center. The click goes to whatever is
/// drawn there, so the element's window comes to the front first, unless
/// its process already has the foreground (an open menu would close).
fn click_center(id: &str, el: &IUIAutomationElement) -> Result<(), CmdError> {
    let hwnd = super::window::parse_id(id.split('/').next().unwrap_or_default())?;
    if !super::window::in_front(hwnd) {
        super::window::focus(hwnd)?;
    }
    let (x, y, w, h) = unsafe { element_rect(el) };
    super::mouse::click(MouseButton::Left, false, Some((x + w / 2, y + h / 2)))
}

pub fn click(id: &str) -> Result<(), CmdError> {
    click_center(id, &resolve(id)?)
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
    }
    click_center(id, &el)
}

/// Apps may apply a change a moment after accepting it, so polls for up to
/// half a second before believing it didn't happen.
fn settle(done: impl Fn() -> bool) -> bool {
    for _ in 0..20 {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    done()
}

fn os_err(e: windows::core::Error) -> CmdError {
    CmdError::os(e.to_string())
}

pub fn check(id: &str, on: bool) -> Result<(), CmdError> {
    let el = resolve(id)?;
    let state = || unsafe { element_state(&el) };
    if state().1 == Some(on) {
        return Ok(());
    }
    let unchanged = || CmdError::os("the control accepted the change but did not switch");
    unsafe {
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
            // A three-state box takes up to two steps to get there. Wait for
            // each step to land, or a slow app would get an extra one.
            for _ in 0..3 {
                let before = (state().1, state().2);
                pattern.Toggle().map_err(os_err)?;
                if !settle(|| (state().1, state().2) != before) {
                    return Err(unchanged());
                }
                if state().1 == Some(on) {
                    return Ok(());
                }
            }
            return Err(unchanged());
        }
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
            if !on {
                return Err(CmdError::invalid_args(
                    "a radio button or tab can't be turned off directly; turn on another one instead",
                ));
            }
            pattern.Select().map_err(os_err)?;
            return if settle(|| state().1 == Some(true)) { Ok(()) } else { Err(unchanged()) };
        }
    }
    if state().1.is_none() && !state().2 {
        return Err(CmdError::unsupported("this element doesn't report a checked state"));
    }
    invoke(id)?;
    if settle(|| state().1 == Some(on)) { Ok(()) } else { Err(unchanged()) }
}

unsafe fn select_item(item: &IUIAutomationElement) -> Result<(), CmdError> {
    unsafe {
        let pattern = item
            .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
            .map_err(|_| CmdError::unsupported("this element can't be selected"))?;
        pattern.Select().map_err(os_err)?;
        let selected = || pattern.CurrentIsSelected().is_ok_and(|s| s.as_bool());
        if settle(selected) { Ok(()) } else { Err(CmdError::os("the item accepted the selection but isn't selected")) }
    }
}

/// The selectable item under `container` with that name. A dropdown may only
/// fill its list a moment after opening, so this keeps looking for a second.
unsafe fn find_selectable(container: &IUIAutomationElement, wanted: &str) -> Result<IUIAutomationElement, CmdError> {
    unsafe {
        let automation = automation()?;
        let condition = automation
            .CreatePropertyCondition(UIA_IsSelectionItemPatternAvailablePropertyId, &VARIANT::from(true))
            .map_err(os_err)?;
        let wanted_lower = wanted.to_lowercase();
        let mut names = Vec::new();
        for _ in 0..10 {
            let items = container.FindAll(TreeScope_Descendants, &condition).map_err(os_err)?;
            names.clear();
            let mut partial = None;
            for i in 0..items.Length().unwrap_or(0) {
                let Ok(item) = items.GetElement(i) else { continue };
                let name = element_name(&item);
                let lower = name.to_lowercase();
                if lower == wanted_lower {
                    return Ok(item);
                }
                if partial.is_none() && lower.contains(&wanted_lower) {
                    partial = Some(item);
                }
                names.push(name);
            }
            if let Some(item) = partial {
                return Ok(item);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(CmdError::not_found(format!("no item \"{wanted}\" here; it has: {}", names.join(", "))))
    }
}

pub fn select(id: &str, item: Option<&str>) -> Result<(), CmdError> {
    let el = resolve(id)?;
    unsafe {
        let Some(wanted) = item else { return select_item(&el) };
        // Only a closed dropdown is opened, and it's closed again afterwards.
        let opened = el
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
            .ok()
            .filter(|p| {
                p.CurrentExpandCollapseState().is_ok_and(|s| s == ExpandCollapseState_Collapsed) && p.Expand().is_ok()
            });
        let result = find_selectable(&el, wanted).and_then(|found| select_item(&found));
        // Asked while it's open: closed, a dropdown's items may all be hidden.
        let hidden = result.is_err() && no_selectable_items(&el)?;
        if let Some(pattern) = opened {
            let _ = pattern.Collapse();
        }
        match result {
            Err(e) if e.code == "not_found" && hidden && element_state(&el).0.is_some() => {
                select_by_value(id, &el, wanted)
            }
            other => other,
        }
    }
}

unsafe fn no_selectable_items(el: &IUIAutomationElement) -> Result<bool, CmdError> {
    unsafe {
        let condition = automation()?
            .CreatePropertyCondition(UIA_IsSelectionItemPatternAvailablePropertyId, &VARIANT::from(true))
            .map_err(os_err)?;
        let items = el.FindAll(TreeScope_Descendants, &condition).map_err(os_err)?;
        Ok(items.Length().unwrap_or(0) == 0)
    }
}

/// For a dropdown that doesn't expose its items, as WinForms ones don't: set
/// the text if it takes that, otherwise walk its values with the arrow keys,
/// which works without opening it. The app sees each value go by.
unsafe fn select_by_value(id: &str, el: &IUIAutomationElement, wanted: &str) -> Result<(), CmdError> {
    unsafe {
        let pattern = el
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .map_err(|_| CmdError::unsupported("this dropdown shows neither its items nor its value"))?;
        let value = || pattern.CurrentValue().map(|b| b.to_string()).unwrap_or_default();
        if !pattern.CurrentIsReadOnly().is_ok_and(|r| r.as_bool())
            && pattern.SetValue(&BSTR::from(wanted)).is_ok()
            && settle(|| value().eq_ignore_ascii_case(wanted))
        {
            return Ok(());
        }
        super::window::focus(super::window::parse_id(id.split('/').next().unwrap_or_default())?)?;
        el.SetFocus().map_err(os_err)?;
        let start = value();
        let press = |key: &str| -> Result<(), CmdError> {
            let before = value();
            super::keyboard::key(key, 1)?;
            settle(|| value() != before);
            Ok(())
        };
        press("home")?;
        let mut values = vec![value()];
        while values.len() < 500 {
            press("down")?;
            let now = value();
            if values.last() == Some(&now) {
                break;
            }
            values.push(now);
        }
        let lower = wanted.to_lowercase();
        let target = values
            .iter()
            .position(|v| v.to_lowercase() == lower)
            .or_else(|| values.iter().position(|v| v.to_lowercase().contains(&lower)));
        // Back where it was if the item isn't there.
        let index = target.or_else(|| values.iter().position(|v| *v == start)).unwrap_or(0);
        press("home")?;
        for _ in 0..index {
            press("down")?;
        }
        if target.is_none() {
            return Err(CmdError::not_found(format!("no item \"{wanted}\" here; it has: {}", values.join(", "))));
        }
        if value() == values[index] {
            Ok(())
        } else {
            Err(CmdError::os("the dropdown did not reach that item"))
        }
    }
}

pub fn expand(id: &str, open: bool) -> Result<(), CmdError> {
    let el = resolve(id)?;
    let what = if open { "expand" } else { "collapse" };
    unsafe {
        let pattern = el
            .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
            .map_err(|_| CmdError::unsupported(format!("this element can't {what}")))?;
        let state = || pattern.CurrentExpandCollapseState();
        if state().is_ok_and(|s| s == ExpandCollapseState_LeafNode) {
            return Err(CmdError::invalid_args(format!("this element has nothing to {what}")));
        }
        let target = if open { ExpandCollapseState_Expanded } else { ExpandCollapseState_Collapsed };
        if state().is_ok_and(|s| s == target) {
            return Ok(());
        }
        if open { pattern.Expand() } else { pattern.Collapse() }.map_err(os_err)?;
        if settle(|| state().is_ok_and(|s| s == target)) {
            Ok(())
        } else {
            Err(CmdError::os(format!("the element accepted the {what} but its state did not change")))
        }
    }
}

/// The first line only: a .NET app adds its whole stack trace.
fn refused(value: f64, e: windows::core::Error) -> CmdError {
    let message = e.message();
    let reason = message.lines().next().unwrap_or_default();
    if e.code() == E_INVALIDARG {
        CmdError::invalid_args(format!("the control refused {value}: {reason}"))
    } else {
        CmdError::os(format!("the control refused {value}: {reason}"))
    }
}

/// Sets a number through the RangeValue pattern, or the Value pattern where
/// the app only has that, and returns what the control ended up with: a
/// slider may snap to its steps.
pub fn set_value(id: &str, value: f64) -> Result<f64, CmdError> {
    let el = resolve(id)?;
    unsafe {
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId) {
            let (min, max) = (pattern.CurrentMinimum().map_err(os_err)?, pattern.CurrentMaximum().map_err(os_err)?);
            if value < min || value > max {
                return Err(CmdError::invalid_args(format!("{value} is outside this control's range, {min} to {max}")));
            }
            pattern.SetValue(value).map_err(|e| refused(value, e))?;
            let now = || pattern.CurrentValue().unwrap_or(f64::NAN);
            settle(|| now() == value);
            return pattern.CurrentValue().map_err(os_err);
        }
        if let Ok(pattern) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            pattern.SetValue(&BSTR::from(value.to_string())).map_err(|e| refused(value, e))?;
            let now = || pattern.CurrentValue().ok().and_then(|b| b.to_string().trim().parse::<f64>().ok());
            settle(|| now() == Some(value));
            return now().ok_or_else(|| {
                CmdError::os("the control took the value but doesn't report a number; check it with `element list`")
            });
        }
    }
    Err(CmdError::unsupported("this element has no value to set"))
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
