use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

pub type HachimiGetApiFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitResult {
    Error = 0,
    Ok = 1,
}

type PresentCallback = unsafe extern "C" fn(swapchain: *mut c_void, userdata: *mut c_void);

struct ApiFns {
    log: unsafe extern "C" fn(level: i32, target: *const c_char, message: *const c_char),
    hachimi_register_present_callback:
        unsafe extern "C" fn(Option<PresentCallback>, *mut c_void) -> bool,
}

static API: OnceLock<ApiFns> = OnceLock::new();

fn resolve(get_api: HachimiGetApiFn, name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    unsafe { get_api(c.as_ptr()) }
}

macro_rules! req {
    ($get:expr, $name:expr) => {{
        let p = resolve($get, $name);
        if p.is_null() {
            return false;
        }
        unsafe { std::mem::transmute(p) }
    }};
}

pub fn init(get_api: HachimiGetApiFn) -> bool {
    let fns = ApiFns {
        log: req!(get_api, "log"),
        hachimi_register_present_callback: req!(get_api, "hachimi_register_present_callback"),
    };
    API.set(fns).is_ok()
}

fn api() -> &'static ApiFns {
    API.get().expect("api not initialized")
}

pub fn log_info(msg: &str) {
    log(3, msg);
}
pub fn log_warn(msg: &str) {
    log(2, msg);
}

fn log(level: i32, msg: &str) {
    let Ok(target) = CString::new("LaunchOverlay") else {
        return;
    };
    let Ok(message) = CString::new(msg) else {
        return;
    };
    unsafe {
        (api().log)(level, target.as_ptr(), message.as_ptr());
    }
}

pub fn register_present_callback(cb: PresentCallback) -> bool {
    unsafe { (api().hachimi_register_present_callback)(Some(cb), std::ptr::null_mut()) }
}