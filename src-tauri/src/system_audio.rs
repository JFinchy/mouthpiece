//! System audio capture via macOS 14.4+ CoreAudio Process Tap
//! (`CATapDescription` + `AudioHardwareCreateProcessTap`).
//!
//! Captures all remote meeting audio playing through the Mac's speakers or
//! headphones without requiring macOS Screen Recording permission or a virtual
//! audio driver. Excludes Mouthpiece's own process ID from the tap.
//!
//! On macOS older than 14.4, or if the tap can't be created,
//! [`SystemAudioTap::start`] returns `Err` and Meeting Mode records the mic
//! only. Note that a *denied* system-audio permission does not fail here:
//! CoreAudio still creates the tap and simply delivers silence. The right
//! channel then stays dead air, and transcription falls back to the mic-only
//! path (see `has_non_zero_audio` and `meeting::transcribe_file`).

use crate::audio::AudioChunk;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// Checks whether the current macOS version supports CoreAudio Process Taps
/// reliably (`macOS >= 14.4`).
pub fn is_supported_os_version(major: isize, minor: isize) -> bool {
    major > 14 || (major == 14 && minor >= 4)
}

/// Returns true if the current system is macOS 14.4 or newer.
#[cfg(target_os = "macos")]
pub fn is_available() -> bool {
    use objc2_foundation::NSProcessInfo;
    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    is_supported_os_version(version.majorVersion, version.minorVersion)
}

#[cfg(not(target_os = "macos"))]
pub fn is_available() -> bool {
    false
}

/// Converts raw bytes from a CoreAudio `AudioBuffer` into `f32` samples.
/// Supports 32-bit float PCM (`is_float = true`) and 16-bit signed integer PCM
/// (`is_float = false`, `bits_per_channel = 16`).
pub fn decode_pcm_bytes(bytes: &[u8], is_float: bool, bits_per_channel: u32) -> Vec<f32> {
    if is_float && bits_per_channel == 32 {
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .map(|s| if s.is_finite() { s } else { 0.0 })
            .collect()
    } else if !is_float && bits_per_channel == 16 {
        bytes
            .chunks_exact(2)
            .map(|b| i16::from_ne_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
            .collect()
    } else {
        Vec::new()
    }
}

/// Computes the RMS level of a slice of `f32` audio samples.
pub fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Pure watchdog rule: returns true when the CoreAudio Process Tap IOProc has
/// stopped advancing `frames_delivered` for 2 consecutive watchdog ticks (~6s)
/// and should be torn down and recreated.
pub fn should_restart_tap(prev_frames: u64, cur_frames: u64, stalled_ticks: &mut u32) -> bool {
    if cur_frames == prev_frames {
        *stalled_ticks += 1;
        if *stalled_ticks >= 2 {
            *stalled_ticks = 0;
            return true;
        }
    } else {
        *stalled_ticks = 0;
    }
    false
}

/// Active CoreAudio Process Tap + Aggregate Device session streaming system
/// audio chunks into a channel. Includes a watchdog that automatically
/// recreates the tap if macOS stalls the IOProc after sleep/wake or an audio
/// output device switch. Stops and tears down the tap when dropped.
pub struct SystemAudioTap {
    level_bits: Arc<AtomicU32>,
    non_zero_seen: Arc<AtomicBool>,
    frames_delivered: Arc<AtomicU64>,
    #[cfg(target_os = "macos")]
    stop_flag: Arc<AtomicBool>,
    #[cfg(target_os = "macos")]
    inner: Arc<std::sync::Mutex<Option<macos_tap::ActiveTap>>>,
    #[cfg(target_os = "macos")]
    watchdog: Option<std::thread::JoinHandle<()>>,
}

impl SystemAudioTap {
    /// Starts a CoreAudio Process Tap that streams system output audio into `tx`.
    #[cfg(target_os = "macos")]
    pub fn start(tx: Sender<AudioChunk>) -> Result<Self, String> {
        if !is_available() {
            return Err("System audio tap requires macOS 14.4 or newer.".into());
        }
        let level_bits = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let non_zero_seen = Arc::new(AtomicBool::new(false));
        let frames_delivered = Arc::new(AtomicU64::new(0));
        let tap = macos_tap::ActiveTap::start(
            tx.clone(),
            Arc::clone(&level_bits),
            Arc::clone(&non_zero_seen),
            Arc::clone(&frames_delivered),
        )?;
        let inner = Arc::new(std::sync::Mutex::new(Some(tap)));
        let stop_flag = Arc::new(AtomicBool::new(false));

        let wd_inner = Arc::clone(&inner);
        let wd_stop = Arc::clone(&stop_flag);
        let wd_level = Arc::clone(&level_bits);
        let wd_nonzero = Arc::clone(&non_zero_seen);
        let wd_frames = Arc::clone(&frames_delivered);
        let watchdog = std::thread::spawn(move || {
            let mut prev_frames = wd_frames.load(Ordering::Relaxed);
            let mut stalled_ticks = 0u32;
            while !wd_stop.load(Ordering::Relaxed) {
                let mut new_rate = None;
                for tick in 1..=30 {
                    if wd_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if tick % 5 == 0 {
                        let guard = wd_inner
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        new_rate = guard.as_ref().and_then(|t| t.rate_changed());
                        if new_rate.is_some() {
                            break;
                        }
                    }
                }
                let cur_frames = wd_frames.load(Ordering::Relaxed);
                if cur_frames == prev_frames {
                    wd_level.store(0.0f32.to_bits(), Ordering::Relaxed);
                }
                let stalled = new_rate.is_none()
                    && should_restart_tap(prev_frames, cur_frames, &mut stalled_ticks);
                if stalled || new_rate.is_some() {
                    match new_rate {
                        Some(rate) => crate::applog!(
                            "system_audio: output format changed to {rate} Hz; restarting tap"
                        ),
                        None => crate::applog!(
                            "system_audio: tap IOProc stalled at {cur_frames} frames; restarting tap"
                        ),
                    }
                    stalled_ticks = 0;
                    let mut guard = wd_inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if wd_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(old) = guard.take() {
                        old.stop_and_destroy();
                    }
                    match macos_tap::ActiveTap::start(
                        tx.clone(),
                        Arc::clone(&wd_level),
                        Arc::clone(&wd_nonzero),
                        Arc::clone(&wd_frames),
                    ) {
                        Ok(new_tap) => *guard = Some(new_tap),
                        Err(err) => {
                            crate::applog!("system_audio: tap restart failed: {err}");
                        }
                    }
                }
                prev_frames = wd_frames.load(Ordering::Relaxed);
            }
        });

        Ok(Self {
            level_bits,
            non_zero_seen,
            frames_delivered,
            stop_flag,
            inner,
            watchdog: Some(watchdog),
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start(_tx: Sender<AudioChunk>) -> Result<Self, String> {
        Err("System audio tap is only supported on macOS 14.4+.".into())
    }

    /// Current RMS level of the system audio stream (for combining with the
    /// mic level meter in the widget).
    pub fn current_level(&self) -> f32 {
        f32::from_bits(self.level_bits.load(Ordering::Relaxed))
    }

    /// True once at least one non-zero audio sample has arrived from the
    /// system tap.
    pub fn has_non_zero_audio(&self) -> bool {
        self.non_zero_seen.load(Ordering::Relaxed)
    }

    /// Total sample frames delivered by the tap IOProc so far.
    #[allow(dead_code)]
    pub fn frames_delivered(&self) -> u64 {
        self.frames_delivered.load(Ordering::Relaxed)
    }
}

impl Drop for SystemAudioTap {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.stop_flag.store(true, Ordering::Relaxed);
            if let Some(handle) = self.watchdog.take() {
                let _ = handle.join();
            }
            let mut guard = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(inner) = guard.take() {
                inner.stop_and_destroy();
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod macos_tap {
    use super::*;
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use coreaudio_sys::{
        kAudioHardwareNoError, kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeInput, kAudioObjectSystemObject, AudioBuffer, AudioBufferList,
        AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID,
        AudioDeviceStart, AudioDeviceStop, AudioObjectGetPropertyData, AudioObjectID,
        AudioObjectPropertyAddress, AudioStreamBasicDescription, AudioTimeStamp, OSStatus,
    };
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::{NSArray, NSNumber, NSString};
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::null;

    const FOURCC_PID_TO_PROCESS: u32 = u32::from_be_bytes(*b"id2p");
    const FOURCC_TAP_FORMAT: u32 = u32::from_be_bytes(*b"tfmt");
    const FOURCC_STREAM_FORMAT: u32 = u32::from_be_bytes(*b"sfmt");
    const FORMAT_FLAG_FLOAT: u32 = 1 << 0;
    const FORMAT_FLAG_NON_INTERLEAVED: u32 = 1 << 5;

    static NEXT_TAP_SEQ: AtomicU64 = AtomicU64::new(1);

    type CreateProcessTapFn =
        unsafe extern "C" fn(desc: *const c_void, out_tap: *mut AudioObjectID) -> OSStatus;
    type DestroyProcessTapFn = unsafe extern "C" fn(tap: AudioObjectID) -> OSStatus;
    type CreateAggregateDeviceFn =
        unsafe extern "C" fn(desc: *const c_void, out_device: *mut AudioObjectID) -> OSStatus;
    type DestroyAggregateDeviceFn = unsafe extern "C" fn(device: AudioObjectID) -> OSStatus;

    unsafe fn resolve_symbol<T>(name: &[u8]) -> Option<T> {
        let sym = unsafe { libc_dlsym(name.as_ptr() as *const i8) };
        if sym.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute_copy(&sym) })
        }
    }

    extern "C" {
        #[link_name = "dlsym"]
        fn dlsym_raw(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
    }

    unsafe fn libc_dlsym(symbol: *const i8) -> *mut c_void {
        // RTLD_DEFAULT on macOS is -2isize as *mut c_void
        unsafe { dlsym_raw((-2isize) as *mut c_void, symbol) }
    }

    fn current_process_audio_object_id() -> Option<AudioObjectID> {
        let mut pid = std::process::id() as i32;
        let address = AudioObjectPropertyAddress {
            mSelector: FOURCC_PID_TO_PROCESS,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut obj_id: AudioObjectID = 0;
        let mut data_size = size_of::<AudioObjectID>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &address,
                size_of::<i32>() as u32,
                &mut pid as *mut i32 as *const c_void,
                &mut data_size,
                &mut obj_id as *mut AudioObjectID as *mut c_void,
            )
        };
        if status == kAudioHardwareNoError as OSStatus && obj_id != 0 {
            Some(obj_id)
        } else {
            None
        }
    }

    fn query_stream_format(
        object_id: AudioObjectID,
        selector: u32,
        scope: u32,
    ) -> Option<AudioStreamBasicDescription> {
        let address = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut desc = AudioStreamBasicDescription::default();
        let mut size = size_of::<AudioStreamBasicDescription>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &address,
                0,
                null(),
                &mut size,
                &mut desc as *mut AudioStreamBasicDescription as *mut c_void,
            )
        };
        if status == kAudioHardwareNoError as OSStatus && desc.mSampleRate > 0.0 {
            Some(desc)
        } else {
            None
        }
    }

    struct IoCallbackContext {
        tx: Sender<AudioChunk>,
        sample_rate: u32,
        channels: u16,
        is_float: bool,
        is_non_interleaved: bool,
        bits_per_channel: u32,
        level_bits: Arc<AtomicU32>,
        non_zero_seen: Arc<AtomicBool>,
        frames_delivered: Arc<AtomicU64>,
    }

    unsafe extern "C" fn tap_io_proc(
        _in_device: AudioObjectID,
        _in_now: *const AudioTimeStamp,
        in_input_data: *const AudioBufferList,
        _in_input_time: *const AudioTimeStamp,
        _out_output_data: *mut AudioBufferList,
        _in_output_time: *const AudioTimeStamp,
        in_client_data: *mut c_void,
    ) -> OSStatus {
        if in_input_data.is_null() || in_client_data.is_null() {
            return 0;
        }
        let ctx = unsafe { &*(in_client_data as *const IoCallbackContext) };
        let list = unsafe { &*in_input_data };
        let num_buffers = list.mNumberBuffers as usize;
        if num_buffers == 0 {
            return 0;
        }

        let buffers_ptr =
            unsafe { std::ptr::addr_of!((*in_input_data).mBuffers) as *const AudioBuffer };
        let buffers: &[AudioBuffer] =
            unsafe { std::slice::from_raw_parts(buffers_ptr, num_buffers) };

        let mut samples = Vec::new();
        let mut out_channels = ctx.channels.max(1);

        if ctx.is_non_interleaved && num_buffers > 1 {
            // Downmix non-interleaved channel buffers directly into mono.
            let mut channel_vecs = Vec::with_capacity(num_buffers);
            for buf in buffers {
                if buf.mData.is_null() || buf.mDataByteSize == 0 {
                    continue;
                }
                let bytes = unsafe {
                    std::slice::from_raw_parts(buf.mData as *const u8, buf.mDataByteSize as usize)
                };
                let ch = decode_pcm_bytes(bytes, ctx.is_float, ctx.bits_per_channel);
                if !ch.is_empty() {
                    channel_vecs.push(ch);
                }
            }
            if let Some(first) = channel_vecs.first() {
                let len = first.len();
                let inv = 1.0 / channel_vecs.len() as f32;
                samples.resize(len, 0.0);
                for ch in &channel_vecs {
                    for (dst, &s) in samples.iter_mut().zip(ch.iter()) {
                        *dst += s * inv;
                    }
                }
                out_channels = 1;
            }
        } else {
            let buf = &buffers[0];
            if !buf.mData.is_null() && buf.mDataByteSize > 0 {
                let bytes = unsafe {
                    std::slice::from_raw_parts(buf.mData as *const u8, buf.mDataByteSize as usize)
                };
                samples = decode_pcm_bytes(bytes, ctx.is_float, ctx.bits_per_channel);
                if buf.mNumberChannels > 0 {
                    out_channels = buf.mNumberChannels as u16;
                }
            }
        }

        if samples.is_empty() {
            return 0;
        }

        let rms = rms_level(&samples);
        ctx.level_bits.store(rms.to_bits(), Ordering::Relaxed);
        if rms > 1e-5 {
            ctx.non_zero_seen.store(true, Ordering::Relaxed);
        }
        let frames = samples.len() / (out_channels.max(1) as usize);
        ctx.frames_delivered
            .fetch_add(frames as u64, Ordering::Relaxed);

        let _ = ctx.tx.send(AudioChunk {
            samples,
            channels: out_channels,
            sample_rate: ctx.sample_rate,
        });
        0
    }

    pub struct ActiveTap {
        tap_id: AudioObjectID,
        device_id: AudioObjectID,
        io_proc_id: AudioDeviceIOProcID,
        ctx_ptr: *mut IoCallbackContext,
        destroy_tap_fn: DestroyProcessTapFn,
        destroy_agg_fn: DestroyAggregateDeviceFn,
        /// The rate chunks are labelled with (fixed in the callback context).
        sample_rate: u32,
    }

    /// The format the IOProc's buffers actually arrive in. With drift
    /// compensation on, the tap is resampled to the aggregate device's
    /// clock, so its input stream format is authoritative; the tap's own
    /// format is only a fallback.
    fn effective_format(
        device_id: AudioObjectID,
        tap_id: AudioObjectID,
    ) -> Option<AudioStreamBasicDescription> {
        query_stream_format(
            device_id,
            FOURCC_STREAM_FORMAT,
            kAudioObjectPropertyScopeInput,
        )
        .or_else(|| query_stream_format(tap_id, FOURCC_TAP_FORMAT, kAudioObjectPropertyScopeGlobal))
    }

    // Safety: ActiveTap holds CoreAudio IDs and a heap-pinned callback context
    // that is only freed after `AudioDeviceStop` and `AudioDeviceDestroyIOProcID`.
    unsafe impl Send for ActiveTap {}

    impl ActiveTap {
        pub fn start(
            tx: Sender<AudioChunk>,
            level_bits: Arc<AtomicU32>,
            non_zero_seen: Arc<AtomicBool>,
            frames_delivered: Arc<AtomicU64>,
        ) -> Result<Self, String> {
            let create_tap_fn: CreateProcessTapFn =
                unsafe { resolve_symbol(b"AudioHardwareCreateProcessTap\0") }
                    .ok_or("AudioHardwareCreateProcessTap symbol not available")?;
            let destroy_tap_fn: DestroyProcessTapFn =
                unsafe { resolve_symbol(b"AudioHardwareDestroyProcessTap\0") }
                    .ok_or("AudioHardwareDestroyProcessTap symbol not available")?;
            let create_agg_fn: CreateAggregateDeviceFn =
                unsafe { resolve_symbol(b"AudioHardwareCreateAggregateDevice\0") }
                    .ok_or("AudioHardwareCreateAggregateDevice symbol not available")?;
            let destroy_agg_fn: DestroyAggregateDeviceFn =
                unsafe { resolve_symbol(b"AudioHardwareDestroyAggregateDevice\0") }
                    .ok_or("AudioHardwareDestroyAggregateDevice symbol not available")?;

            let cls = AnyClass::get(c"CATapDescription")
                .ok_or("CATapDescription class not available on this macOS version")?;

            let (desc, tap_uuid) = objc2::rc::autoreleasepool(|_| -> Result<_, String> {
                let excluded: Retained<NSArray<NSNumber>> = match current_process_audio_object_id()
                {
                    Some(obj_id) => {
                        let num = NSNumber::new_u32(obj_id);
                        NSArray::from_retained_slice(&[num])
                    }
                    None => NSArray::new(),
                };

                unsafe {
                    let raw: *mut AnyObject = msg_send![cls, alloc];
                    if raw.is_null() {
                        return Err("Failed to allocate CATapDescription".into());
                    }
                    let desc_ptr: *mut AnyObject =
                        msg_send![raw, initMonoGlobalTapButExcludeProcesses: &*excluded];
                    let desc =
                        Retained::from_raw(desc_ptr).ok_or("Failed to init CATapDescription")?;

                    let tap_name = NSString::from_str("Mouthpiece Meeting System Audio Tap");
                    let _: () = msg_send![&*desc, setName: &*tap_name];
                    let _: () = msg_send![&*desc, setPrivate: Bool::YES];
                    let mute_behavior: isize = 0; // CATapUnmuted
                    let _: () = msg_send![&*desc, setMuteBehavior: mute_behavior];

                    let uuid_obj: *mut AnyObject = msg_send![&*desc, UUID];
                    if uuid_obj.is_null() {
                        return Err("CATapDescription returned null UUID".into());
                    }
                    let uuid_str_ptr: *mut NSString = msg_send![uuid_obj, UUIDString];
                    let uuid_str = Retained::retain(uuid_str_ptr)
                        .ok_or("CATapDescription UUIDString was null")?
                        .to_string();
                    Ok((desc, uuid_str))
                }
            })?;

            let mut tap_id: AudioObjectID = 0;
            let tap_status =
                unsafe { create_tap_fn(Retained::as_ptr(&desc) as *const c_void, &mut tap_id) };
            if tap_status != kAudioHardwareNoError as OSStatus || tap_id == 0 {
                return Err(format!(
                    "AudioHardwareCreateProcessTap failed with OSStatus {tap_status}"
                ));
            }

            let seq = NEXT_TAP_SEQ.fetch_add(1, Ordering::Relaxed);
            let agg_uid = format!("com.mouthpiece.system-tap.{}.{seq}", std::process::id());
            let sub_tap_dict: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::from_static_string("uid"),
                    CFString::new(&tap_uuid).as_CFType(),
                ),
                (
                    CFString::from_static_string("drift"),
                    CFBoolean::true_value().as_CFType(),
                ),
            ]);
            let tap_list = CFArray::from_CFTypes(&[sub_tap_dict]);
            let agg_dict: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::from_static_string("name"),
                    CFString::from_static_string("Mouthpiece System Tap").as_CFType(),
                ),
                (
                    CFString::from_static_string("uid"),
                    CFString::new(&agg_uid).as_CFType(),
                ),
                (
                    CFString::from_static_string("private"),
                    CFBoolean::true_value().as_CFType(),
                ),
                (
                    CFString::from_static_string("stacked"),
                    CFBoolean::false_value().as_CFType(),
                ),
                (
                    CFString::from_static_string("tapautostart"),
                    CFBoolean::true_value().as_CFType(),
                ),
                (CFString::from_static_string("taps"), tap_list.as_CFType()),
            ]);

            let mut device_id: AudioObjectID = 0;
            let agg_status = unsafe {
                create_agg_fn(
                    agg_dict.as_concrete_TypeRef() as *const c_void,
                    &mut device_id,
                )
            };
            if agg_status != kAudioHardwareNoError as OSStatus || device_id == 0 {
                unsafe {
                    let _ = destroy_tap_fn(tap_id);
                }
                return Err(format!(
                    "AudioHardwareCreateAggregateDevice failed with OSStatus {agg_status}"
                ));
            }

            // Default to 48 kHz mono f32 if neither format can be read.
            let asbd = effective_format(device_id, tap_id);
            let (sample_rate, channels, is_float, is_non_interleaved, bits_per_channel) = match asbd
            {
                Some(fmt) => (
                    fmt.mSampleRate.round().max(8000.0) as u32,
                    (fmt.mChannelsPerFrame as u16).max(1),
                    // The flag alone: 32-bit *integer* PCM decoded as float is noise.
                    (fmt.mFormatFlags & FORMAT_FLAG_FLOAT) != 0,
                    (fmt.mFormatFlags & FORMAT_FLAG_NON_INTERLEAVED) != 0,
                    if fmt.mBitsPerChannel > 0 {
                        fmt.mBitsPerChannel
                    } else {
                        32
                    },
                ),
                None => (48_000, 1, true, false, 32),
            };

            let ctx = Box::new(IoCallbackContext {
                tx,
                sample_rate,
                channels,
                is_float,
                is_non_interleaved,
                bits_per_channel,
                level_bits,
                non_zero_seen,
                frames_delivered,
            });
            let ctx_ptr = Box::into_raw(ctx);

            let mut io_proc_id: AudioDeviceIOProcID = None;
            let proc_status = unsafe {
                AudioDeviceCreateIOProcID(
                    device_id,
                    Some(tap_io_proc),
                    ctx_ptr as *mut c_void,
                    &mut io_proc_id,
                )
            };
            if proc_status != kAudioHardwareNoError as OSStatus || io_proc_id.is_none() {
                unsafe {
                    drop(Box::from_raw(ctx_ptr));
                    let _ = destroy_agg_fn(device_id);
                    let _ = destroy_tap_fn(tap_id);
                }
                return Err(format!(
                    "AudioDeviceCreateIOProcID failed with OSStatus {proc_status}"
                ));
            }

            let start_status = unsafe { AudioDeviceStart(device_id, io_proc_id) };
            if start_status != kAudioHardwareNoError as OSStatus {
                unsafe {
                    let _ = AudioDeviceDestroyIOProcID(device_id, io_proc_id);
                    drop(Box::from_raw(ctx_ptr));
                    let _ = destroy_agg_fn(device_id);
                    let _ = destroy_tap_fn(tap_id);
                }
                return Err(format!(
                    "AudioDeviceStart for system tap failed with OSStatus {start_status}"
                ));
            }

            crate::applog!(
                "system_audio: started CoreAudio Process Tap ({} Hz, {} ch)",
                sample_rate,
                channels
            );

            Ok(Self {
                tap_id,
                device_id,
                io_proc_id,
                ctx_ptr,
                destroy_tap_fn,
                destroy_agg_fn,
                sample_rate,
            })
        }

        /// The new rate, if the device's format changed since the tap was
        /// built (e.g. Bluetooth headphones dropping to headset mode when a
        /// call app opens the mic, or the output device switching). Chunks
        /// would otherwise keep the stale rate label, and the system channel
        /// would play at the wrong speed and drift.
        pub fn rate_changed(&self) -> Option<u32> {
            let rate = effective_format(self.device_id, self.tap_id)?
                .mSampleRate
                .round()
                .max(8000.0) as u32;
            (rate != self.sample_rate).then_some(rate)
        }

        pub fn stop_and_destroy(self) {
            unsafe {
                let _ = AudioDeviceStop(self.device_id, self.io_proc_id);
                let _ = AudioDeviceDestroyIOProcID(self.device_id, self.io_proc_id);
                drop(Box::from_raw(self.ctx_ptr));
                let _ = (self.destroy_agg_fn)(self.device_id);
                let _ = (self.destroy_tap_fn)(self.tap_id);
            }
            crate::applog!("system_audio: stopped CoreAudio Process Tap");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_version_gate_requires_macos_14_4_or_newer() {
        assert!(!is_supported_os_version(13, 6));
        assert!(!is_supported_os_version(14, 0));
        assert!(!is_supported_os_version(14, 3));
        assert!(is_supported_os_version(14, 4));
        assert!(is_supported_os_version(14, 5));
        assert!(is_supported_os_version(15, 0));
        assert!(is_supported_os_version(26, 0));
    }

    #[test]
    fn decode_pcm_bytes_handles_f32_and_i16_streams() {
        let f32_samples = [0.25f32, -0.5f32, f32::NAN, 1.0f32];
        let mut raw_f32 = Vec::new();
        for s in f32_samples {
            raw_f32.extend_from_slice(&s.to_ne_bytes());
        }
        let decoded = decode_pcm_bytes(&raw_f32, true, 32);
        assert_eq!(decoded, vec![0.25, -0.5, 0.0, 1.0]);

        let i16_samples = [i16::MAX / 2, -(i16::MAX / 2)];
        let mut raw_i16 = Vec::new();
        for s in i16_samples {
            raw_i16.extend_from_slice(&s.to_ne_bytes());
        }
        let decoded_i16 = decode_pcm_bytes(&raw_i16, false, 16);
        assert_eq!(decoded_i16.len(), 2);
        assert!((decoded_i16[0] - 0.5).abs() < 1e-3);
        assert!((decoded_i16[1] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn rms_level_computes_expected_energy() {
        assert_eq!(rms_level(&[]), 0.0);
        let level = rms_level(&[0.5, -0.5, 0.5, -0.5]);
        assert!((level - 0.5).abs() < 1e-6);
    }

    #[test]
    fn watchdog_restarts_after_two_stalled_ticks_and_resets_on_progress() {
        let mut stalled = 0;
        assert!(!should_restart_tap(100, 200, &mut stalled));
        assert_eq!(stalled, 0);

        // First stalled tick: does not restart yet.
        assert!(!should_restart_tap(200, 200, &mut stalled));
        assert_eq!(stalled, 1);

        // Progress resumes: resets counter.
        assert!(!should_restart_tap(200, 350, &mut stalled));
        assert_eq!(stalled, 0);

        // Two consecutive stalled ticks: triggers tap restart and resets counter.
        assert!(!should_restart_tap(350, 350, &mut stalled));
        assert!(should_restart_tap(350, 350, &mut stalled));
        assert_eq!(stalled, 0);
    }
}
