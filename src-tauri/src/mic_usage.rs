//! Which other processes are capturing from a microphone or playing audio
//! right now — the signal ambient listening uses to stay out of calls, and
//! meeting auto-detection uses to notice when a call starts or ends. Checks
//! actual audio use via CoreAudio's per-process list rather than guessing
//! from which apps are open: Zoom often runs idle in the background, while
//! Google Meet runs inside a browser.

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use coreaudio_sys::{
    kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
    kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyIsRunningOutput,
    kAudioProcessPropertyPID, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectID, AudioObjectPropertyAddress,
};
use std::ffi::c_void;

/// Another process currently holding an active CoreAudio input and/or output
/// stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioProcessHolder {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub is_input: bool,
    pub is_output: bool,
}

fn address(selector: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn read_u32(object: AudioObjectID, selector: u32) -> Option<u32> {
    let addr = address(selector);
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut value as *mut u32 as *mut c_void,
        )
    };
    (status == 0).then_some(value)
}

fn read_cfstring(object: AudioObjectID, selector: u32) -> Option<String> {
    let addr = address(selector);
    let mut value: CFStringRef = std::ptr::null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut value as *mut CFStringRef as *mut c_void,
        )
    };
    if status != 0 || value.is_null() {
        return None;
    }
    let s = unsafe { CFString::wrap_under_create_rule(value) }.to_string();
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn process_objects() -> Option<Vec<AudioObjectID>> {
    let addr = address(kAudioHardwarePropertyProcessObjectList);
    let mut size: u32 = 0;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }

    let count = size as usize / std::mem::size_of::<AudioObjectID>();
    let mut processes: Vec<AudioObjectID> = vec![0; count];
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            processes.as_mut_ptr() as *mut c_void,
        )
    };
    if status != 0 {
        return None;
    }
    processes.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
    Some(processes)
}

/// Other processes currently capturing audio input or playing audio output,
/// with their bundle identifiers (macOS 14+). Excludes this process itself.
pub fn active_audio_holders() -> Option<Vec<AudioProcessHolder>> {
    let processes = process_objects()?;
    let own = std::process::id() as i32;
    Some(
        processes
            .into_iter()
            .filter_map(|p| {
                let is_input =
                    read_u32(p, kAudioProcessPropertyIsRunningInput).is_some_and(|r| r != 0);
                let is_output =
                    read_u32(p, kAudioProcessPropertyIsRunningOutput).is_some_and(|r| r != 0);
                if !is_input && !is_output {
                    return None;
                }
                let pid = read_u32(p, kAudioProcessPropertyPID)? as i32;
                if pid == own {
                    return None;
                }
                let bundle_id = read_cfstring(p, kAudioProcessPropertyBundleID);
                Some(AudioProcessHolder {
                    pid,
                    bundle_id,
                    is_input,
                    is_output,
                })
            })
            .collect(),
    )
}

/// PIDs of other processes currently capturing audio input, or `None` if
/// the OS can't say (the per-process API needs macOS 14+).
pub fn other_processes_using_mic() -> Option<Vec<i32>> {
    Some(
        active_audio_holders()?
            .into_iter()
            .filter(|h| h.is_input)
            .map(|h| h.pid)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real CoreAudio call. Can't assert who's using the mic
    /// on a dev machine, only that the query works and never reports this
    /// test process itself.
    #[test]
    fn query_succeeds_and_excludes_this_process() {
        let result = other_processes_using_mic();
        eprintln!("mic users: {result:?}");
        if let Some(pids) = result {
            assert!(!pids.contains(&(std::process::id() as i32)));
        }
    }

    #[test]
    fn active_audio_holders_succeeds_and_excludes_this_process() {
        let result = active_audio_holders();
        eprintln!("audio holders: {result:?}");
        if let Some(holders) = result {
            let own = std::process::id() as i32;
            assert!(holders
                .iter()
                .all(|h| h.pid != own && (h.is_input || h.is_output)));
        }
    }
}
