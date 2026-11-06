//! Virtual desktops. Windows only documents `IVirtualDesktopManager`, which
//! can't list or switch desktops, and can only move the caller's own windows.
//! Everything here goes through Explorer's internal interfaces instead, whose
//! GUIDs and method order come from MScholtes/VirtualDesktop and change
//! between builds. These are Windows 10's (1809 to 22H2). On another build
//! Explorer doesn't know the interface IDs, so the first query fails and this
//! reports "unsupported" rather than calling a method that isn't there.

#![allow(non_snake_case)]

use super::window::{check_access, WindowHandle};
use crate::output::CmdError;
use core::ffi::c_void;
use serde::Serialize;
use windows::core::{interface, IUnknown, Interface, GUID, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{BOOL, E_NOINTERFACE, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IServiceProvider, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::UI::Shell::Common::IObjectArray;

#[derive(Serialize)]
pub struct WorkspaceInfo {
    /// The desktop's GUID, as Windows keeps it.
    pub id: String,
    pub name: String,
    /// 1-based position, as shown in Task View.
    pub index: u32,
    pub current: bool,
}

const CLSID_IMMERSIVE_SHELL: GUID = GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239);
const CLSID_VIRTUAL_DESKTOP_MANAGER_INTERNAL: GUID = GUID::from_u128(0xc5e0cdca_7b6e_41b2_9fc4_d93975cc467b);

#[interface("ff72ffdd-be7e-43fc-9c03-ad81681e88e4")]
unsafe trait IVirtualDesktop: windows::core::IUnknown {
    fn IsViewVisible(&self, view: *mut c_void, visible: *mut BOOL) -> HRESULT;
    fn GetId(&self, id: *mut GUID) -> HRESULT;
}

/// Only the methods up to the last one used; the rest of the vtable follows.
#[interface("f31574d6-b682-4cdc-bd56-1827860abec6")]
unsafe trait IVirtualDesktopManagerInternal: windows::core::IUnknown {
    fn GetCount(&self, count: *mut u32) -> HRESULT;
    fn MoveViewToDesktop(&self, view: *mut c_void, desktop: *mut c_void) -> HRESULT;
    fn CanViewMoveDesktops(&self, view: *mut c_void, can: *mut BOOL) -> HRESULT;
    fn GetCurrentDesktop(&self, desktop: *mut *mut c_void) -> HRESULT;
    fn GetDesktops(&self, desktops: *mut *mut c_void) -> HRESULT;
    fn GetAdjacentDesktop(&self, from: *mut c_void, direction: u32, desktop: *mut *mut c_void) -> HRESULT;
    fn SwitchDesktop(&self, desktop: *mut c_void) -> HRESULT;
    fn CreateDesktop(&self, desktop: *mut *mut c_void) -> HRESULT;
    fn RemoveDesktop(&self, desktop: *mut c_void, fallback: *mut c_void) -> HRESULT;
}

/// The same, plus naming, from Windows 10 2004 on.
#[interface("0f3a72b0-4566-487e-9a33-4ed302f6d6ce")]
unsafe trait IVirtualDesktopManagerInternal2: windows::core::IUnknown {
    fn GetCount(&self, count: *mut u32) -> HRESULT;
    fn MoveViewToDesktop(&self, view: *mut c_void, desktop: *mut c_void) -> HRESULT;
    fn CanViewMoveDesktops(&self, view: *mut c_void, can: *mut BOOL) -> HRESULT;
    fn GetCurrentDesktop(&self, desktop: *mut *mut c_void) -> HRESULT;
    fn GetDesktops(&self, desktops: *mut *mut c_void) -> HRESULT;
    fn GetAdjacentDesktop(&self, from: *mut c_void, direction: u32, desktop: *mut *mut c_void) -> HRESULT;
    fn SwitchDesktop(&self, desktop: *mut c_void) -> HRESULT;
    fn CreateDesktop(&self, desktop: *mut *mut c_void) -> HRESULT;
    fn RemoveDesktop(&self, desktop: *mut c_void, fallback: *mut c_void) -> HRESULT;
    fn FindDesktop(&self, id: *const GUID, desktop: *mut *mut c_void) -> HRESULT;
    fn Unknown1(&self, desktop: *mut c_void, a: *mut *mut c_void, b: *mut *mut c_void) -> HRESULT;
    /// `name` is an HSTRING the callee only borrows.
    fn SetName(&self, desktop: *mut c_void, name: *mut c_void) -> HRESULT;
}

/// Only the methods up to `GetViewForHwnd`.
#[interface("1841c6d7-4f9d-42c0-af41-8747538f10e5")]
unsafe trait IApplicationViewCollection: windows::core::IUnknown {
    fn GetViews(&self, views: *mut *mut c_void) -> HRESULT;
    fn GetViewsByZOrder(&self, views: *mut *mut c_void) -> HRESULT;
    fn GetViewsByAppUserModelId(&self, id: PCWSTR, views: *mut *mut c_void) -> HRESULT;
    fn GetViewForHwnd(&self, hwnd: HWND, view: *mut *mut c_void) -> HRESULT;
}

struct Shell {
    provider: IServiceProvider,
    manager: IVirtualDesktopManagerInternal,
}

struct Desktop {
    desktop: IVirtualDesktop,
    id: GUID,
}

/// Lowercase, without braces, like the UUIDs KWin gives on Linux.
fn guid_string(id: &GUID) -> String {
    format!("{id:?}").to_lowercase()
}

fn os_error(what: &str, e: windows::core::Error) -> CmdError {
    CmdError::os(format!("{what}: {}", e.message()))
}

fn shell() -> Result<Shell, CmdError> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let provider: IServiceProvider = CoCreateInstance(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER)
            .map_err(|e| os_error("could not reach Explorer's shell, which owns the virtual desktops", e))?;
        let manager = provider
            .QueryService::<IVirtualDesktopManagerInternal>(&CLSID_VIRTUAL_DESKTOP_MANAGER_INTERNAL)
            .map_err(|e| {
                if e.code() == E_NOINTERFACE {
                    CmdError::unsupported(
                        "virtual desktops use an undocumented interface that changes between Windows builds, and guictl only knows Windows 10's",
                    )
                } else {
                    os_error("could not reach the virtual desktop manager", e)
                }
            })?;
        Ok(Shell { provider, manager })
    }
}

/// Whether this build of Windows has the virtual desktop interface guictl knows.
pub fn available() -> bool {
    shell().is_ok()
}

impl Shell {
    fn desktops(&self) -> Result<Vec<Desktop>, CmdError> {
        unsafe {
            let mut raw = std::ptr::null_mut();
            self.manager.GetDesktops(&mut raw).ok().map_err(|e| os_error("could not list the virtual desktops", e))?;
            let array = IObjectArray::from_raw(raw);
            let count = array.GetCount().map_err(|e| os_error("could not count the virtual desktops", e))?;
            (0..count)
                .map(|i| {
                    let desktop: IVirtualDesktop =
                        array.GetAt(i).map_err(|e| os_error("could not read a virtual desktop", e))?;
                    let mut id = GUID::zeroed();
                    desktop.GetId(&mut id).ok().map_err(|e| os_error("could not read a virtual desktop's id", e))?;
                    Ok(Desktop { desktop, id })
                })
                .collect()
        }
    }

    fn current(&self) -> Result<GUID, CmdError> {
        unsafe {
            let mut raw = std::ptr::null_mut();
            self.manager
                .GetCurrentDesktop(&mut raw)
                .ok()
                .map_err(|e| os_error("could not read the current virtual desktop", e))?;
            let desktop = IVirtualDesktop::from_raw(raw);
            let mut id = GUID::zeroed();
            desktop.GetId(&mut id).ok().map_err(|e| os_error("could not read a virtual desktop's id", e))?;
            Ok(id)
        }
    }

    /// `id` is a GUID from `list`, with or without braces, or a 1-based index.
    fn find(&self, id: &str) -> Result<Desktop, CmdError> {
        let mut desktops = self.desktops()?;
        let at = position(&desktops, id)?;
        Ok(desktops.swap_remove(at))
    }
}

/// Where in `desktops` the one `id` names is.
fn position(desktops: &[Desktop], id: &str) -> Result<usize, CmdError> {
    let wanted = id.trim().trim_start_matches('{').trim_end_matches('}').to_lowercase();
    let at = match wanted.parse::<usize>() {
        Ok(index) => (1..=desktops.len()).contains(&index).then(|| index - 1),
        Err(_) => desktops.iter().position(|d| guid_string(&d.id) == wanted),
    };
    at.ok_or_else(|| CmdError::not_found(format!("no workspace with id {id}; `workspace list` shows the ones there are")))
}

/// The name the person gave the desktop in Task View, or Windows's own
/// "Desktop N" when they never renamed it, which Windows doesn't store.
fn name(id: &GUID, index: u32) -> String {
    let key: Vec<u16> = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\VirtualDesktops\\Desktops\\{{{id:?}}}\0"
    )
    .encode_utf16()
    .collect();
    let value: Vec<u16> = "Name\0".encode_utf16().collect();
    let mut buf = [0u16; 256];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    match String::from_utf16_lossy(&buf[..len]) {
        name if read.is_ok() && !name.is_empty() => name,
        _ => format!("Desktop {index}"),
    }
}

pub fn list() -> Result<Vec<WorkspaceInfo>, CmdError> {
    let shell = shell()?;
    let current = shell.current()?;
    Ok(shell
        .desktops()?
        .iter()
        .zip(1..)
        .map(|(d, index)| WorkspaceInfo {
            id: guid_string(&d.id),
            name: name(&d.id, index),
            index,
            current: d.id == current,
        })
        .collect())
}

pub fn switch(id: &str) -> Result<(), CmdError> {
    let shell = shell()?;
    let target = shell.find(id)?;
    unsafe { shell.manager.SwitchDesktop(target.desktop.as_raw()) }
        .ok()
        .map_err(|e| os_error("could not switch virtual desktop", e))
}

/// Its windows move to the desktop before it, or after it for the first, and
/// so does the person if they were on it.
pub fn remove(id: &str) -> Result<(), CmdError> {
    let shell = shell()?;
    let desktops = shell.desktops()?;
    let at = position(&desktops, id)?;
    if desktops.len() == 1 {
        return Err(CmdError::invalid_args("can't remove the only workspace"));
    }
    let fallback = if at == 0 { 1 } else { at - 1 };
    unsafe { shell.manager.RemoveDesktop(desktops[at].desktop.as_raw(), desktops[fallback].desktop.as_raw()) }
        .ok()
        .map_err(|e| os_error("could not remove the virtual desktop", e))
}

pub fn create(name: Option<&str>) -> Result<WorkspaceInfo, CmdError> {
    let shell = shell()?;
    // Asked for first, so a build without naming fails before a desktop exists.
    let namer = match name {
        Some(_) => Some(unsafe {
            shell
                .provider
                .QueryService::<IVirtualDesktopManagerInternal2>(&CLSID_VIRTUAL_DESKTOP_MANAGER_INTERNAL)
                .map_err(|_| {
                    CmdError::unsupported("naming a workspace needs Windows 10 version 2004 or later; leave out --name")
                })?
        }),
        None => None,
    };
    let desktop = unsafe {
        let mut raw = std::ptr::null_mut();
        shell.manager.CreateDesktop(&mut raw).ok().map_err(|e| os_error("could not create a virtual desktop", e))?;
        IVirtualDesktop::from_raw(raw)
    };
    if let (Some(namer), Some(name)) = (namer, name) {
        let name = HSTRING::from(name);
        unsafe { namer.SetName(desktop.as_raw(), std::mem::transmute_copy(&name)) }
            .ok()
            .map_err(|e| os_error("created the virtual desktop but could not name it", e))?;
    }
    let mut id = GUID::zeroed();
    unsafe { desktop.GetId(&mut id) }.ok().map_err(|e| os_error("could not read a virtual desktop's id", e))?;
    list()?
        .into_iter()
        .find(|w| w.id == guid_string(&id))
        .ok_or_else(|| CmdError::os("created a virtual desktop, but it isn't in the list"))
}

pub fn send_window(window: WindowHandle, id: &str) -> Result<(), CmdError> {
    check_access(window, "This window")?;
    let shell = shell()?;
    let target = shell.find(id)?;
    unsafe {
        let views: IApplicationViewCollection = shell
            .provider
            .QueryService(&IApplicationViewCollection::IID)
            .map_err(|e| os_error("could not reach Explorer's list of windows", e))?;
        let mut raw = std::ptr::null_mut();
        views.GetViewForHwnd(window, &mut raw).ok().map_err(|_| {
            CmdError::unsupported(
                "Windows doesn't count this window as one that belongs to a virtual desktop (it has no taskbar entry)",
            )
        })?;
        let view = IUnknown::from_raw(raw);
        shell
            .manager
            .MoveViewToDesktop(view.as_raw(), target.desktop.as_raw())
            .ok()
            .map_err(|e| os_error("could not move the window to that virtual desktop", e))
    }
}
