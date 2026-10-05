#![allow(non_snake_case)]

use crate::output::CmdError;
use serde::Serialize;
use windows::core::{Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eAll, eCapture, eCommunications, eConsole, eMultimedia, eRender, ERole, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, IMMEndpoint, ISimpleAudioVolume,
    MMDeviceEnumerator, DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED,
    DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    STGM_READ,
};

#[derive(Serialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub direction: &'static str,
    pub state: &'static str,
}

/// Active, disabled, and unplugged devices - the same set the classic Sound
/// Control Panel shows with "Show Disabled Devices" / "Show Disconnected
/// Devices" both checked. Not-present devices (no hardware at all) are left
/// out since there's nothing addressable about them.
const ALL_RELEVANT_STATES: DEVICE_STATE =
    DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0);

/// `IPolicyConfig`/`CPolicyConfigClient` aren't part of the public Windows
/// SDK - no header, no documentation, no entry in the Win32 metadata this
/// crate's bindings are generated from. This is the reverse-engineered
/// interface (GUID and vtable order) that most third-party "set default
/// audio device" tools have relied on since Windows 7; it has proven stable
/// across Windows versions in practice, but Microsoft could change it
/// without notice since it isn't a contract they publish.
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn GetMixFormat(&self, device_id: PCWSTR, format: *mut *mut core::ffi::c_void) -> HRESULT;
    fn GetDeviceFormat(
        &self,
        device_id: PCWSTR,
        default: i32,
        format: *mut *mut core::ffi::c_void,
    ) -> HRESULT;
    fn ResetDeviceFormat(&self, device_id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(
        &self,
        device_id: PCWSTR,
        endpoint_format: *mut core::ffi::c_void,
        mix_format: *mut core::ffi::c_void,
    ) -> HRESULT;
    fn GetProcessingPeriod(
        &self,
        device_id: PCWSTR,
        default: i32,
        default_period: *mut i64,
        minimum_period: *mut i64,
    ) -> HRESULT;
    fn SetProcessingPeriod(&self, device_id: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, device_id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
    fn SetShareMode(&self, device_id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
    fn GetPropertyValue(
        &self,
        device_id: PCWSTR,
        key: *const core::ffi::c_void,
        value: *mut core::ffi::c_void,
    ) -> HRESULT;
    fn SetPropertyValue(
        &self,
        device_id: PCWSTR,
        key: *const core::ffi::c_void,
        value: *const core::ffi::c_void,
    ) -> HRESULT;
    fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: i32) -> HRESULT;
}

fn device_enumerator() -> Result<IMMDeviceEnumerator, CmdError> {
    unsafe {
        // Ignore the result: in this one-shot process COM is uninitialized, so
        // S_OK is expected; a prior init on this thread (S_FALSE) is fine too.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| CmdError::os(format!("could not start the audio device enumerator: {e}")))
    }
}

fn default_render_device() -> Result<IMMDevice, CmdError> {
    let enumerator = device_enumerator()?;
    unsafe {
        enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(|e| CmdError::not_found(format!("no default playback device: {e}")))
    }
}

fn endpoint_volume() -> Result<IAudioEndpointVolume, CmdError> {
    let device = default_render_device()?;
    unsafe {
        device
            .Activate::<IAudioEndpointVolume>(CLSCTX_INPROC_SERVER, None)
            .map_err(|e| CmdError::os(e.to_string()))
    }
}

/// Finds the audio session owned by a process named `app` and returns its
/// per-session volume control. Each playback stream gets its own session,
/// so a multi-window app with several streams will match whichever session
/// enumerates first.
fn session_simple_volume(app: &str) -> Result<ISimpleAudioVolume, CmdError> {
    let device = default_render_device()?;
    unsafe {
        let manager: IAudioSessionManager2 = device
            .Activate(CLSCTX_INPROC_SERVER, None)
            .map_err(|e| CmdError::os(e.to_string()))?;
        let sessions = manager
            .GetSessionEnumerator()
            .map_err(|e| CmdError::os(e.to_string()))?;
        let count = sessions.GetCount().map_err(|e| CmdError::os(e.to_string()))?;
        for i in 0..count {
            let control = sessions.GetSession(i).map_err(|e| CmdError::os(e.to_string()))?;
            let control2: IAudioSessionControl2 = match control.cast() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let pid = control2.GetProcessId().unwrap_or(0);
            if pid != 0 && super::window::process_name(pid).eq_ignore_ascii_case(app) {
                return control2
                    .cast::<ISimpleAudioVolume>()
                    .map_err(|e| CmdError::os(e.to_string()));
            }
        }
    }
    Err(CmdError::not_found(format!("no audio session for app '{app}'")))
}

pub fn get_volume(app: Option<&str>) -> Result<u32, CmdError> {
    let level = unsafe {
        match app {
            Some(app) => session_simple_volume(app)?.GetMasterVolume(),
            None => endpoint_volume()?.GetMasterVolumeLevelScalar(),
        }
    }
    .map_err(|e| CmdError::os(e.to_string()))?;
    Ok((level * 100.0).round() as u32)
}

pub fn set_volume(app: Option<&str>, level: u32) -> Result<(), CmdError> {
    let scalar = (level.min(100) as f32) / 100.0;
    unsafe {
        match app {
            Some(app) => session_simple_volume(app)?.SetMasterVolume(scalar, std::ptr::null()),
            None => endpoint_volume()?.SetMasterVolumeLevelScalar(scalar, std::ptr::null()),
        }
    }
    .map_err(|e| CmdError::os(e.to_string()))
}

pub fn set_mute(app: Option<&str>, mute: bool) -> Result<(), CmdError> {
    unsafe {
        match app {
            Some(app) => session_simple_volume(app)?.SetMute(mute, std::ptr::null()),
            None => endpoint_volume()?.SetMute(mute, std::ptr::null()),
        }
    }
    .map_err(|e| CmdError::os(e.to_string()))
}

pub fn list_devices() -> Result<Vec<AudioDeviceInfo>, CmdError> {
    let enumerator = device_enumerator()?;
    unsafe {
        let collection = enumerator
            .EnumAudioEndpoints(eAll, ALL_RELEVANT_STATES)
            .map_err(|e| CmdError::os(e.to_string()))?;
        let count = collection.GetCount().map_err(|e| CmdError::os(e.to_string()))?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = collection.Item(i).map_err(|e| CmdError::os(e.to_string()))?;

            let id = match device.GetId() {
                Ok(ptr) => {
                    let s = ptr.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(ptr.0 as *const _));
                    s
                }
                Err(_) => String::new(),
            };

            let name = device
                .OpenPropertyStore(STGM_READ)
                .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
                .map(|pv| pv.to_string())
                .unwrap_or_default();

            let direction = device
                .cast::<IMMEndpoint>()
                .and_then(|ep| ep.GetDataFlow())
                .map(|flow| if flow == eCapture { "input" } else { "output" })
                .unwrap_or("output");

            let state = match device.GetState() {
                Ok(s) if s == DEVICE_STATE_ACTIVE => "active",
                Ok(s) if s == DEVICE_STATE_DISABLED => "disabled",
                Ok(s) if s == DEVICE_STATE_UNPLUGGED => "unplugged",
                _ => "unknown",
            };

            out.push(AudioDeviceInfo { id, name, direction, state });
        }
        Ok(out)
    }
}

/// Enables or disables an endpoint via the same undocumented
/// `IPolicyConfig::SetEndpointVisibility` call the classic Sound Control
/// Panel's "Enable"/"Disable" context menu items use. A disabled device
/// still shows up in `list_devices` (state "disabled") so it can be
/// re-enabled later.
pub fn set_enabled(device_id: &str, enabled: bool) -> Result<(), CmdError> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let policy_config: IPolicyConfig =
            CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_INPROC_SERVER).map_err(|e| {
                CmdError::os(format!("could not start the audio policy config service: {e}"))
            })?;
        let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
        policy_config
            .SetEndpointVisibility(PCWSTR(wide.as_ptr()), enabled as i32)
            .ok()
            .map_err(|e| CmdError::os(format!("SetEndpointVisibility failed: {e}")))
    }
}

/// Sets `device_id` (an id from `list_devices`) as the default endpoint for
/// all three roles (console, multimedia, communications) - matching what
/// the Settings app's "Set as default" action does. Works for both output
/// and input devices; `SetDefaultEndpoint` doesn't care about direction.
pub fn set_default_device(device_id: &str) -> Result<(), CmdError> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let policy_config: IPolicyConfig =
            CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_INPROC_SERVER).map_err(|e| {
                CmdError::os(format!("could not start the audio policy config service: {e}"))
            })?;
        let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
        let id = PCWSTR(wide.as_ptr());
        for role in [eConsole, eMultimedia, eCommunications] {
            policy_config
                .SetDefaultEndpoint(id, role)
                .ok()
                .map_err(|e| CmdError::os(format!("SetDefaultEndpoint failed: {e}")))?;
        }
    }
    Ok(())
}
