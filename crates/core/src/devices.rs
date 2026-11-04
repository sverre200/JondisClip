//! Audio input devices for the microphone picker.

use serde::Serialize;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eConsole,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputDevice {
    /// Stable Windows endpoint ID (what `microphone_device` stores).
    pub id: String,
    /// Friendly name, e.g. "Microphone (Arctis 7 Chat)".
    pub name: String,
    /// The current Windows default recording device.
    pub is_default: bool,
}

fn id_of(d: &IMMDevice) -> Option<String> {
    unsafe {
        let p = d.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

fn name_of(d: &IMMDevice) -> Option<String> {
    unsafe {
        let store = d.OpenPropertyStore(STGM_READ).ok()?;
        let v = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let s = v.to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Active (plugged-in, enabled) microphones, Windows default first.
pub fn list_inputs() -> windows::core::Result<Vec<InputDevice>> {
    unsafe {
        // Fine if COM is already initialised on this thread.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let default_id = e
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .ok()
            .and_then(|d| id_of(&d));
        let coll = e.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        let mut out = Vec::new();
        for i in 0..coll.GetCount()? {
            let d = coll.Item(i)?;
            let Some(id) = id_of(&d) else { continue };
            let name = name_of(&d).unwrap_or_else(|| "Microphone".into());
            out.push(InputDevice {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            });
        }
        out.sort_by_key(|d| !d.is_default);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    /// Lists this PC's microphones (needs real audio hardware):
    /// `cargo test -p jondisclip-core list_real_inputs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn list_real_inputs() {
        let list = super::list_inputs().unwrap();
        for d in &list {
            println!("{} {}", if d.is_default { "*" } else { " " }, d.name);
        }
        assert!(list.iter().filter(|d| d.is_default).count() <= 1);
        if let Some(first) = list.first() {
            assert!(
                first.is_default || !list.iter().any(|d| d.is_default),
                "default sorts first"
            );
        }
    }
}
