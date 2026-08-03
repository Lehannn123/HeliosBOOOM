mod api;
mod overlay;
mod stats;

use api::{HachimiGetApiFn, InitResult};

#[no_mangle]
pub unsafe extern "C" fn hachimi_init_v3(get_api: HachimiGetApiFn, version: i32) -> InitResult {
    if version < 3 || !api::init(get_api) {
        return InitResult::Error;
    }

    api::log_info("launch_overlay: loading");

    if !api::register_present_callback(overlay::on_present) {
        api::log_warn("launch_overlay: failed to register present callback");
        return InitResult::Error;
    }

    api::log_info("launch_overlay: ready");
    InitResult::Ok
}

#[no_mangle]
pub unsafe extern "C" fn trigger_overlay() {
    overlay::trigger();
}

#[no_mangle]
pub unsafe extern "C" fn hachimi_init(
    _vtable: *const std::ffi::c_void,
    _version: i32,
) -> InitResult {
    InitResult::Error
}