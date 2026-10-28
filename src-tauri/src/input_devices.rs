//! What kind of mic each input device is (the Mac's own, Bluetooth
//! headphones, USB, virtual), so a missing or silent mic falls back to the
//! one most likely to work. Bluetooth headphones are the worst stand-in:
//! their mic drops music to call quality, and when that switch fails (music
//! already playing) they send pure silence.

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use coreaudio_sys::{
    kAudioDevicePropertyDeviceNameCFString, kAudioDevicePropertyStreams,
    kAudioDevicePropertyTransportType, kAudioDeviceTransportTypeAggregate,
    kAudioDeviceTransportTypeAutoAggregate, kAudioDeviceTransportTypeBluetooth,
    kAudioDeviceTransportTypeBluetoothLE, kAudioDeviceTransportTypeBuiltIn,
    kAudioDeviceTransportTypeVirtual, kAudioHardwarePropertyDevices,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectSystemObject, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
};
use std::ffi::c_void;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    BuiltIn,
    Bluetooth,
    /// USB, Thunderbolt, Continuity and the like: real mics.
    Other,
    /// Virtual and aggregate devices (Zoom's, loopback drivers): never
    /// picked automatically, since they're often silent by design.
    Virtual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    pub kind: Kind,
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn data_size(object: AudioObjectID, addr: &AudioObjectPropertyAddress) -> Option<u32> {
    let mut size: u32 = 0;
    let status =
        unsafe { AudioObjectGetPropertyDataSize(object, addr, 0, std::ptr::null(), &mut size) };
    (status == 0).then_some(size)
}

fn read<T: Default>(object: AudioObjectID, addr: &AudioObjectPropertyAddress) -> Option<T> {
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut value as *mut T as *mut c_void,
        )
    };
    (status == 0).then_some(value)
}

fn device_ids() -> Vec<AudioObjectID> {
    let addr = address(
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
    );
    let Some(mut size) = data_size(kAudioObjectSystemObject, &addr) else {
        return Vec::new();
    };
    let mut ids: Vec<AudioObjectID> = vec![0; size as usize / std::mem::size_of::<AudioObjectID>()];
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr() as *mut c_void,
        )
    };
    if status != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
    ids
}

/// The same name cpal reports, so the two can be matched.
fn device_name(id: AudioObjectID) -> Option<String> {
    let addr = address(
        kAudioDevicePropertyDeviceNameCFString,
        kAudioObjectPropertyScopeGlobal,
    );
    let raw: usize = read(id, &addr)?;
    if raw == 0 {
        return None;
    }
    let name = unsafe { CFString::wrap_under_create_rule(raw as CFStringRef) };
    Some(name.to_string())
}

fn kind_of(transport: u32) -> Kind {
    #[allow(non_upper_case_globals)]
    match transport {
        kAudioDeviceTransportTypeBuiltIn => Kind::BuiltIn,
        kAudioDeviceTransportTypeBluetooth | kAudioDeviceTransportTypeBluetoothLE => {
            Kind::Bluetooth
        }
        kAudioDeviceTransportTypeVirtual
        | kAudioDeviceTransportTypeAggregate
        | kAudioDeviceTransportTypeAutoAggregate => Kind::Virtual,
        _ => Kind::Other,
    }
}

/// Every device that can record, with its kind.
pub fn inputs() -> Vec<Input> {
    let streams = address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
    let transport = address(
        kAudioDevicePropertyTransportType,
        kAudioObjectPropertyScopeGlobal,
    );
    device_ids()
        .into_iter()
        .filter(|&id| data_size(id, &streams).is_some_and(|size| size > 0))
        .filter_map(|id| {
            Some(Input {
                name: device_name(id)?,
                kind: kind_of(read::<u32>(id, &transport).unwrap_or(0)),
            })
        })
        .collect()
}

/// Where to record when the picked mic is missing: the Mac's own mic if
/// the system default is Bluetooth headphones, otherwise `None` (the
/// system default).
pub fn fallback_for_missing(inputs: &[Input], default: Option<&str>) -> Option<String> {
    let default_kind = inputs
        .iter()
        .find(|i| Some(i.name.as_str()) == default)
        .map(|i| i.kind);
    if default_kind != Some(Kind::Bluetooth) {
        return None;
    }
    inputs
        .iter()
        .find(|i| i.kind == Kind::BuiltIn)
        .map(|i| i.name.clone())
}

/// Inputs in the order to suggest them: the Mac's own mic first, then
/// other real mics, then Bluetooth, then virtual devices.
pub fn suggestion_order(inputs: &[Input]) -> Vec<Input> {
    let rank = |kind: Kind| match kind {
        Kind::BuiltIn => 0,
        Kind::Other => 1,
        Kind::Bluetooth => 2,
        Kind::Virtual => 3,
    };
    let mut sorted = inputs.to_vec();
    sorted.sort_by_key(|i| rank(i.kind));
    sorted
}

/// Another mic to try when `dead` sends nothing at all.
pub fn alternative_to(inputs: &[Input], dead: &str) -> Option<String> {
    suggestion_order(inputs)
        .into_iter()
        .find(|i| i.name != dead && i.kind != Kind::Virtual)
        .map(|i| i.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, kind: Kind) -> Input {
        Input {
            name: name.to_string(),
            kind,
        }
    }

    fn desk() -> Vec<Input> {
        vec![
            input("Jake’s AirPods #2", Kind::Bluetooth),
            input("ZoomAudioDevice", Kind::Virtual),
            input("AIRHUG 28", Kind::Other),
            input("MacBook Pro Microphone", Kind::BuiltIn),
        ]
    }

    #[test]
    fn a_missing_mic_falls_back_to_the_mac_instead_of_bluetooth() {
        assert_eq!(
            fallback_for_missing(&desk(), Some("Jake’s AirPods #2")).as_deref(),
            Some("MacBook Pro Microphone")
        );
    }

    #[test]
    fn a_non_bluetooth_default_is_kept() {
        assert_eq!(fallback_for_missing(&desk(), Some("AIRHUG 28")), None);
        assert_eq!(
            fallback_for_missing(&desk(), Some("MacBook Pro Microphone")),
            None
        );
        let no_builtin = vec![input("Jake’s AirPods #2", Kind::Bluetooth)];
        assert_eq!(
            fallback_for_missing(&no_builtin, Some("Jake’s AirPods #2")),
            None
        );
    }

    #[test]
    fn suggestions_put_the_mac_first_and_virtual_devices_last() {
        let names: Vec<_> = suggestion_order(&desk())
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(
            names,
            [
                "MacBook Pro Microphone",
                "AIRHUG 28",
                "Jake’s AirPods #2",
                "ZoomAudioDevice"
            ]
        );
    }

    #[test]
    fn a_dead_mic_is_replaced_by_the_next_real_one() {
        assert_eq!(
            alternative_to(&desk(), "Jake’s AirPods #2").as_deref(),
            Some("MacBook Pro Microphone")
        );
        assert_eq!(
            alternative_to(&desk(), "MacBook Pro Microphone").as_deref(),
            Some("AIRHUG 28")
        );
        let only_virtual = vec![
            input("MacBook Pro Microphone", Kind::BuiltIn),
            input("ZoomAudioDevice", Kind::Virtual),
        ];
        assert_eq!(
            alternative_to(&only_virtual, "MacBook Pro Microphone"),
            None
        );
    }

    /// Exercises the real CoreAudio calls: every Mac laptop has a built-in
    /// mic, and every listed input has a name cpal also reports.
    #[test]
    fn real_inputs_are_listed_with_kinds() {
        let found = inputs();
        eprintln!("inputs: {found:?}");
        let cpal_names = crate::audio::list_device_names();
        for input in &found {
            assert!(
                cpal_names.contains(&input.name),
                "{} isn't a name cpal knows",
                input.name
            );
        }
    }
}
