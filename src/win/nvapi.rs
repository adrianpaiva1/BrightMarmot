//! NVIDIA digital vibrance (DVC) and hue, through `nvapi64.dll` loaded at runtime.
//!
//! Only the functions listed below are ever resolved (spec §8.1). Any non-OK status from a
//! DVC/hue call turns vibrance and hue off for the rest of the run (spec §5.9): getters then
//! return `None` and setters `false`, without retrying.

use crate::vibrance::VibranceInfo;
use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::mem::{size_of, transmute};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

// Function IDs for `nvapi_QueryInterface`.
// - Initialize, GetAssociatedNvidiaDisplayHandle: NVIDIA's public SDK, github.com/NVIDIA/nvapi,
//   `nvapi_interface.h` (signatures from `nvapi.h`).
// - DVC and hue (undocumented): NvAPIWrapper, github.com/falahati/NvAPIWrapper,
//   `NvAPIWrapper/Native/Helpers/FunctionId.cs`; signatures from `Native/Delegates/Display.cs`,
//   layouts from `Native/Display/Structures/PrivateDisplayDVCInfoEx.cs` and
//   `PrivateDisplayHUEInfo.cs`.
const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_GET_ASSOCIATED_DISPLAY_HANDLE: u32 = 0x35C2_9134;
const ID_GET_DVC_INFO_EX: u32 = 0x0E45_002D;
const ID_SET_DVC_LEVEL_EX: u32 = 0x4A82_C2B1;
const ID_GET_HUE_INFO: u32 = 0x95B6_4341;
const ID_SET_HUE_ANGLE: u32 = 0xF5A0_F22C;

const NVAPI_OK: i32 = 0;
/// Output id `0` (NvAPIWrapper's `OutputId.Invalid`): the display the handle refers to.
const OUTPUT_DEFAULT: u32 = 0;

type NvDisplayHandle = *mut c_void;
type QueryInterfaceFn = unsafe extern "C" fn(u32) -> *const c_void;
type InitializeFn = unsafe extern "C" fn() -> i32;
type GetAssociatedHandleFn = unsafe extern "C" fn(*const c_char, *mut NvDisplayHandle) -> i32;
type GetDvcInfoExFn = unsafe extern "C" fn(NvDisplayHandle, u32, *mut DvcInfoEx) -> i32;
type SetDvcLevelExFn = unsafe extern "C" fn(NvDisplayHandle, u32, *mut DvcInfoEx) -> i32;
type GetHueInfoFn = unsafe extern "C" fn(NvDisplayHandle, u32, *mut HueInfo) -> i32;
type SetHueAngleFn = unsafe extern "C" fn(NvDisplayHandle, u32, i32) -> i32;

#[repr(C)]
#[derive(Default)]
struct DvcInfoEx {
    version: u32,
    current: i32,
    min: i32,
    max: i32,
    default: i32,
}

#[repr(C)]
#[derive(Default)]
struct HueInfo {
    version: u32,
    current: i32,
    default: i32,
}

// Sequential 4-byte fields, no padding: the sizes the reference layouts give.
const _: () = assert!(size_of::<DvcInfoEx>() == 20 && size_of::<HueInfo>() == 12);

/// NvAPI struct version: byte size in the low 16 bits, version number (1) above.
const fn version_of<T>() -> u32 {
    size_of::<T>() as u32 | (1 << 16)
}

struct ColourFns {
    get_dvc: GetDvcInfoExFn,
    set_dvc: SetDvcLevelExFn,
    get_hue: GetHueInfoFn,
    set_hue: SetHueAngleFn,
}

pub struct NvApi {
    get_handle: GetAssociatedHandleFn,
    /// `None` when any DVC/hue function is missing from the driver.
    colour: Option<ColourFns>,
    /// The first non-OK DVC/hue status; once set, vibrance and hue stay off for the run.
    failed: Cell<Option<i32>>,
}

/// Resolves `id`; a null pointer means the driver does not export it.
fn query(qi: QueryInterfaceFn, id: u32) -> Option<*const c_void> {
    // SAFETY: nvapi_QueryInterface takes any id and returns null for unknown ones.
    let p = unsafe { qi(id) };
    (!p.is_null()).then_some(p)
}

impl NvApi {
    /// Loads `nvapi64.dll` and initializes NvAPI. The DLL is never unloaded, so the function
    /// pointers stay valid for the whole process.
    pub fn load() -> Option<NvApi> {
        let name: Vec<u16> = "nvapi64.dll\0".encode_utf16().collect();
        // SAFETY: `name` is NUL-terminated and outlives the call.
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return None;
        }
        // SAFETY: `module` is a loaded module and the name is a NUL-terminated C string.
        let qi = unsafe { GetProcAddress(module, c"nvapi_QueryInterface".as_ptr().cast()) }?;
        // SAFETY: nvapi_QueryInterface has the C signature `void* (unsigned int)`.
        let qi: QueryInterfaceFn = unsafe { transmute(qi) };

        // SAFETY (all transmutes below): each pointer was returned for its ID, and the target
        // type is the signature the cited source gives for that ID.
        let init: InitializeFn = unsafe { transmute(query(qi, ID_INITIALIZE)?) };
        // SAFETY: NvAPI_Initialize takes no arguments.
        if unsafe { init() } != NVAPI_OK {
            return None;
        }
        let get_handle: GetAssociatedHandleFn =
            unsafe { transmute(query(qi, ID_GET_ASSOCIATED_DISPLAY_HANDLE)?) };
        let colour = (|| {
            Some(ColourFns {
                get_dvc: unsafe { transmute::<*const c_void, GetDvcInfoExFn>(query(qi, ID_GET_DVC_INFO_EX)?) },
                set_dvc: unsafe { transmute::<*const c_void, SetDvcLevelExFn>(query(qi, ID_SET_DVC_LEVEL_EX)?) },
                get_hue: unsafe { transmute::<*const c_void, GetHueInfoFn>(query(qi, ID_GET_HUE_INFO)?) },
                set_hue: unsafe { transmute::<*const c_void, SetHueAngleFn>(query(qi, ID_SET_HUE_ANGLE)?) },
            })
        })();
        Some(NvApi { get_handle, colour, failed: Cell::new(None) })
    }

    pub fn vibrance_supported(&self) -> bool {
        self.colour.is_some() && self.failed.get().is_none()
    }

    /// The status that switched vibrance/hue off, if any (for the probe's output).
    #[cfg(debug_assertions)]
    pub fn failed_status(&self) -> Option<i32> {
        self.failed.get()
    }

    /// The colour functions, unless missing or switched off by an earlier failure.
    fn colour(&self) -> Option<&ColourFns> {
        if self.failed.get().is_some() { None } else { self.colour.as_ref() }
    }

    /// NVIDIA display handle for a GDI device name. Failure only means this monitor is not
    /// driven by an NVIDIA GPU, so it does not switch anything off.
    fn handle(&self, device: &str) -> Option<NvDisplayHandle> {
        if !device.is_ascii() {
            return None;
        }
        let name = CString::new(device).ok()?;
        let mut h: NvDisplayHandle = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and outlives the call; `h` is a valid out pointer.
        let st = unsafe { (self.get_handle)(name.as_ptr(), &mut h) };
        (st == NVAPI_OK && !h.is_null()).then_some(h)
    }

    /// `Some(())` on OK; otherwise switches vibrance/hue off for the run.
    fn check(&self, status: i32) -> Option<()> {
        if status == NVAPI_OK {
            return Some(());
        }
        if self.failed.get().is_none() {
            self.failed.set(Some(status));
        }
        None
    }

    pub fn get_vibrance(&self, device: &str) -> Option<VibranceInfo> {
        let f = self.colour()?;
        let h = self.handle(device)?;
        let mut info = DvcInfoEx { version: version_of::<DvcInfoEx>(), ..Default::default() };
        // SAFETY: `info` is a correctly versioned DvcInfoEx that the call fills in.
        self.check(unsafe { (f.get_dvc)(h, OUTPUT_DEFAULT, &mut info) })?;
        Some(VibranceInfo { current: info.current, min: info.min, max: info.max, default: info.default })
    }

    pub fn set_vibrance(&self, device: &str, raw: i32) -> bool {
        let Some(f) = self.colour() else { return false };
        let Some(h) = self.handle(device) else { return false };
        let mut info = DvcInfoEx { version: version_of::<DvcInfoEx>(), current: raw, ..Default::default() };
        // SAFETY: `info` is a correctly versioned DvcInfoEx; only `current` is read as the level.
        self.check(unsafe { (f.set_dvc)(h, OUTPUT_DEFAULT, &mut info) }).is_some()
    }

    /// Current hue angle in degrees (0-359).
    pub fn get_hue(&self, device: &str) -> Option<u32> {
        let f = self.colour()?;
        let h = self.handle(device)?;
        let mut info = HueInfo { version: version_of::<HueInfo>(), ..Default::default() };
        // SAFETY: `info` is a correctly versioned HueInfo that the call fills in.
        self.check(unsafe { (f.get_hue)(h, OUTPUT_DEFAULT, &mut info) })?;
        u32::try_from(info.current).ok().filter(|d| *d < 360)
    }

    pub fn set_hue(&self, device: &str, degrees: u32) -> bool {
        if degrees >= 360 {
            return false;
        }
        let Some(f) = self.colour() else { return false };
        let Some(h) = self.handle(device) else { return false };
        // SAFETY: plain value arguments; `degrees < 360` so the cast is lossless.
        self.check(unsafe { (f.set_hue)(h, OUTPUT_DEFAULT, degrees as i32) }).is_some()
    }
}
