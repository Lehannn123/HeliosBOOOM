use crate::api;
use std::ffi::{c_char, c_void, CString};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::PCSTR;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

static LAST_TRIGGER: Mutex<Option<Instant>> = Mutex::new(None);

type DomainGetFn = unsafe extern "C" fn() -> *mut c_void;
type DomainAssemblyOpenFn = unsafe extern "C" fn(domain: *mut c_void, name: *const c_char) -> *mut c_void;
type AssemblyGetImageFn = unsafe extern "C" fn(assembly: *mut c_void) -> *mut c_void;
type ClassFromNameFn = unsafe extern "C" fn(image: *mut c_void, namesp: *const c_char, name: *const c_char) -> *mut c_void;
type ClassGetMethodFromNameFn = unsafe extern "C" fn(class: *mut c_void, name: *const c_char, args_count: i32) -> *mut c_void;
type RuntimeInvokeFn = unsafe extern "C" fn(method: *mut c_void, obj: *mut c_void, params: *mut *mut c_void, exc: *mut *mut c_void) -> *mut c_void;
type ObjectGetClassFn = unsafe extern "C" fn(obj: *mut c_void) -> *mut c_void;

unsafe fn get_il2cpp_export(game_assembly: windows::Win32::Foundation::HMODULE, name: &str) -> *mut c_void {
    let c_name = CString::new(name).unwrap();
    if let Some(proc) = GetProcAddress(game_assembly, PCSTR(c_name.as_ptr() as *const u8)) {
        proc as *mut c_void
    } else {
        std::ptr::null_mut()
    }
}

pub unsafe fn check_career_stats() -> bool {
    // 1. Cooldown Check (throttled to avoid log spam every frame)
    if let Ok(guard) = LAST_TRIGGER.lock() {
        if let Some(last_time) = *guard {
            if last_time.elapsed() < Duration::from_secs(30) {
                return false;
            }
        }
    }

    let module_name: Vec<u16> = "GameAssembly.dll\0".encode_utf16().collect();
    let h_game_assembly = match GetModuleHandleW(windows::core::PCWSTR(module_name.as_ptr())) {
        Ok(h) if !h.is_invalid() => h,
        _ => return false,
    };

    let domain_get_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_domain_get");
    let domain_assembly_open_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_domain_assembly_open");
    let assembly_get_image_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_assembly_get_image");
    let class_from_name_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_class_from_name");
    let class_get_method_from_name_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_class_get_method_from_name");
    let runtime_invoke_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_runtime_invoke");
    let object_get_class_ptr = get_il2cpp_export(h_game_assembly, "il2cpp_object_get_class");

    if domain_get_ptr.is_null()
        || domain_assembly_open_ptr.is_null()
        || assembly_get_image_ptr.is_null()
        || class_from_name_ptr.is_null()
        || class_get_method_from_name_ptr.is_null()
        || runtime_invoke_ptr.is_null()
        || object_get_class_ptr.is_null()
    {
        api::log_warn("launch_overlay: check_career_stats: one or more il2cpp exports not resolved");
        return false;
    }

    let domain_get: DomainGetFn = std::mem::transmute(domain_get_ptr);
    let domain_assembly_open: DomainAssemblyOpenFn = std::mem::transmute(domain_assembly_open_ptr);
    let assembly_get_image: AssemblyGetImageFn = std::mem::transmute(assembly_get_image_ptr);
    let class_from_name: ClassFromNameFn = std::mem::transmute(class_from_name_ptr);
    let class_get_method_from_name: ClassGetMethodFromNameFn = std::mem::transmute(class_get_method_from_name_ptr);
    let runtime_invoke: RuntimeInvokeFn = std::mem::transmute(runtime_invoke_ptr);
    let object_get_class: ObjectGetClassFn = std::mem::transmute(object_get_class_ptr);

    let c = |s: &str| CString::new(s).unwrap();

    let domain = domain_get();
    if domain.is_null() {
        api::log_warn("launch_overlay: check_career_stats: domain_get returned null");
        return false;
    }

    let assembly = domain_assembly_open(domain, c("umamusume.dll").as_ptr());
    if assembly.is_null() {
        api::log_warn("launch_overlay: check_career_stats: umamusume.dll not found");
        return false;
    }

    let image = assembly_get_image(assembly);
    if image.is_null() {
        api::log_warn("launch_overlay: check_career_stats: assembly_get_image returned null");
        return false;
    }

    // Milestone 1: Reached Image
    let work_mgr_class = class_from_name(image, c("Gallop").as_ptr(), c("WorkDataManager").as_ptr());
    if work_mgr_class.is_null() {
        api::log_warn("launch_overlay: check_career_stats: class Gallop.WorkDataManager not found");
        return false;
    }

    let get_instance_m = class_get_method_from_name(work_mgr_class, c("get_Instance").as_ptr(), 0);
    if get_instance_m.is_null() {
        api::log_warn("launch_overlay: check_career_stats: WorkDataManager.get_Instance method not found");
        return false;
    }

    let instance = runtime_invoke(get_instance_m, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
    if instance.is_null() {
        api::log_warn("launch_overlay: check_career_stats: WorkDataManager.Instance is null");
        return false;
    }

    // Milestone 2: Got WorkDataManager instance
    let get_single_m = class_get_method_from_name(work_mgr_class, c("get_SingleMode").as_ptr(), 0);
    if get_single_m.is_null() { 
        api::log_warn("launch_overlay: WorkDataManager.get_SingleMode method not found!");
        return false; 
    }

    let single_mode = runtime_invoke(get_single_m, instance, std::ptr::null_mut(), std::ptr::null_mut());
    if single_mode.is_null() {
        // This means you are on the title screen or main menu (not inside career mode yet)
        api::log_warn("launch_overlay: check_career_stats: get_SingleMode returned null (not in career?)");
        return false;
    }

    // Milestone 3: Got WorkSingleModeData!
    let single_mode_class = object_get_class(single_mode);
    let get_chara_m = class_get_method_from_name(single_mode_class, c("get_Character").as_ptr(), 0);
    if get_chara_m.is_null() {
        api::log_warn("launch_overlay: check_career_stats: WorkSingleModeData.get_Character method not found");
        return false;
    }

    let chara = runtime_invoke(get_chara_m, single_mode, std::ptr::null_mut(), std::ptr::null_mut());
    if chara.is_null() {
        api::log_warn("launch_overlay: check_career_stats: get_Character returned null");
        return false;
    }

    // Milestone 4: Got Character Data!
    let chara_class = object_get_class(chara);
    let stat_methods = ["get_Speed", "get_Stamina", "get_Power", "get_Guts", "get_Wiz"];

    for method_name in stat_methods {
        let method = class_get_method_from_name(chara_class, c(method_name).as_ptr(), 0);
        if !method.is_null() {
            let res = runtime_invoke(method, chara, std::ptr::null_mut(), std::ptr::null_mut());
            if !res.is_null() {
                let stat_val = *(res.add(0x10) as *const i32);

                api::log_info(&format!("launch_overlay: Checked {}: {}", method_name, stat_val));

                if stat_val.to_string().contains("67") {
                    api::log_info(&format!("launch_overlay: Triggered on {} = {}", method_name, stat_val));
                    if let Ok(mut guard) = LAST_TRIGGER.lock() {
                        *guard = Some(Instant::now());
                    }
                    return true;
                }
            }
        }
    }

    false
}