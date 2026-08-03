use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::PCSTR;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

type Il2CppDomainGet = unsafe extern "C" fn() -> *mut c_void;
type Il2CppDomainGetAssemblies = unsafe extern "C" fn(domain: *mut c_void, size: *mut usize) -> *mut *mut c_void;
type Il2CppAssemblyGetImage = unsafe extern "C" fn(assembly: *mut c_void) -> *mut c_void;
type Il2CppClassFromName = unsafe extern "C" fn(image: *mut c_void, namesp: *const c_char, name: *const c_char) -> *mut c_void;
type Il2CppClassGetMethodFromName = unsafe extern "C" fn(klass: *mut c_void, name: *const c_char, args_count: i32) -> *mut c_void;
type Il2CppRuntimeInvoke = unsafe extern "C" fn(method: *mut c_void, obj: *mut c_void, params: *mut *mut c_void, exc: *mut *mut c_void) -> *mut c_void;

struct Il2CppApi {
    domain_get: Il2CppDomainGet,
    domain_get_assemblies: Il2CppDomainGetAssemblies,
    assembly_get_image: Il2CppAssemblyGetImage,
    class_from_name: Il2CppClassFromName,
    class_get_method_from_name: Il2CppClassGetMethodFromName,
    runtime_invoke: Il2CppRuntimeInvoke,
}

static IL2CPP: OnceLock<Option<Il2CppApi>> = OnceLock::new();
static mut LAST_TRIGGER: Option<Instant> = None;
const COOLDOWN: Duration = Duration::from_secs(6);

fn get_il2cpp() -> Option<&'static Il2CppApi> {
    IL2CPP.get_or_init(|| unsafe {
        let module = LoadLibraryA(PCSTR(b"GameAssembly.dll\0".as_ptr())).ok()?;
        macro_rules! get_fn {
            ($name:expr) => {{
                let c_name = CString::new($name).unwrap();
                let addr = GetProcAddress(module, PCSTR(c_name.as_ptr() as *const u8))?;
                std::mem::transmute(addr)
            }};
        }

        Some(Il2CppApi {
            domain_get: get_fn!("il2cpp_domain_get"),
            domain_get_assemblies: get_fn!("il2cpp_domain_get_assemblies"),
            assembly_get_image: get_fn!("il2cpp_assembly_get_image"),
            class_from_name: get_fn!("il2cpp_class_from_name"),
            class_get_method_from_name: get_fn!("il2cpp_class_get_method_from_name"),
            runtime_invoke: get_fn!("il2cpp_runtime_invoke"),
        })
    }).as_ref()
}

pub fn check_career_stats() -> bool {
    unsafe {
        if let Some(last) = LAST_TRIGGER {
            if last.elapsed() < COOLDOWN {
                return false;
            }
        }

        let Some(api) = get_il2cpp() else { return false; };

        let domain = (api.domain_get)();
        if domain.is_null() { return false; }

        let mut size: usize = 0;
        let assemblies = (api.domain_get_assemblies)(domain, &mut size);
        if assemblies.is_null() { return false; }

        let mut image_ptr = std::ptr::null_mut();
        for i in 0..size {
            let assembly = *assemblies.add(i);
            let img = (api.assembly_get_image)(assembly);
            if !img.is_null() {
                image_ptr = img;
                break;
            }
        }
        if image_ptr.is_null() { return false; }

        // Gallop.WorkDataManager
        let gallop_ns = CString::new("Gallop").unwrap();
        let mgr_name = CString::new("WorkDataManager").unwrap();
        let mgr_class = (api.class_from_name)(image_ptr, gallop_ns.as_ptr(), mgr_name.as_ptr());
        if mgr_class.is_null() { return false; }

        // GetSingleMode()
        let get_sm_name = CString::new("GetSingleMode").unwrap();
        let get_sm_method = (api.class_get_method_from_name)(mgr_class, get_sm_name.as_ptr(), 0);
        if get_sm_method.is_null() { return false; }

        let single_mode = (api.runtime_invoke)(get_sm_method, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        if single_mode.is_null() { return false; }

        // GetCharacter()
        let chara_class = (api.class_from_name)(image_ptr, gallop_ns.as_ptr(), CString::new("WorkSingleModeData").unwrap().as_ptr());
        if chara_class.is_null() { return false; }

        let get_chara_method = (api.class_get_method_from_name)(chara_class, CString::new("GetCharacter").unwrap().as_ptr(), 0);
        if get_chara_method.is_null() { return false; }

        let chara = (api.runtime_invoke)(get_chara_method, single_mode, std::ptr::null_mut(), std::ptr::null_mut());
        if chara.is_null() { return false; }

        // Read Stats
        let chara_data_class = (api.class_from_name)(image_ptr, gallop_ns.as_ptr(), CString::new("WorkSingleModeCharaData").unwrap().as_ptr());
        if chara_data_class.is_null() { return false; }

        let stat_methods = ["GetSpeed", "GetStamina", "GetPower", "GetGuts", "GetWiz"];
        for method_name in stat_methods {
            let c_method = CString::new(method_name).unwrap();
            let m = (api.class_get_method_from_name)(chara_data_class, c_method.as_ptr(), 0);
            if !m.is_null() {
                let res = (api.runtime_invoke)(m, chara, std::ptr::null_mut(), std::ptr::null_mut());
                if !res.is_null() {
                    let stat_val = *(res.add(0x10) as *const i32); // Unbox IL2CPP primitive int32
                    if stat_val.to_string().contains("67") {
                        LAST_TRIGGER = Some(Instant::now());
                        return true;
                    }
                }
            }
        }
    }
    false
}