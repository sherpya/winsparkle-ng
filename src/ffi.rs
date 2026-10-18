use crate::{
    settings,
    updater::{self, CheckMode},
};
use std::{
    ffi::{CStr, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
};
pub type VoidCallback = unsafe extern "C" fn();
pub type IntCallback = unsafe extern "C" fn() -> i32;
pub type InstallerCallback = unsafe extern "C" fn(*const u16) -> i32;
#[derive(Clone, Copy, Default)]
pub struct Callbacks {
    pub error: Option<VoidCallback>,
    pub can_shutdown: Option<IntCallback>,
    pub shutdown: Option<VoidCallback>,
    pub found: Option<VoidCallback>,
    pub not_found: Option<VoidCallback>,
    pub cancelled: Option<VoidCallback>,
    pub skipped: Option<VoidCallback>,
    pub postponed: Option<VoidCallback>,
    pub dismissed: Option<VoidCallback>,
    pub installer: Option<InstallerCallback>,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ConfigMethodsRaw {
    pub read: Option<unsafe extern "C" fn(*const c_char, *mut u16, usize, *mut c_void) -> i32>,
    pub write: Option<unsafe extern "C" fn(*const c_char, *const u16, *mut c_void)>,
    pub delete: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    pub data: *mut c_void,
}
#[derive(Clone, Copy)]
pub struct ConfigMethods {
    pub read: unsafe extern "C" fn(*const c_char, *mut u16, usize, *mut c_void) -> i32,
    pub write: unsafe extern "C" fn(*const c_char, *const u16, *mut c_void),
    pub delete: unsafe extern "C" fn(*const c_char, *mut c_void),
    pub data: usize,
}
fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        crate::platform::log("Panic caught at C ABI boundary");
        default
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn panic_is_contained_at_the_ffi_boundary() {
        assert_eq!(
            super::guard(-1, || -> i32 {
                panic!("contained panic");
            }),
            -1
        );
    }
}
pub fn invoke(callback: Option<VoidCallback>) {
    if let Some(callback) = callback {
        // SAFETY: host promises callback remains callable until cleanup and does not throw across C ABI.
        unsafe {
            callback();
        }
    }
}
fn narrow(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: non-null C ABI inputs are valid NUL-terminated UTF-8 strings per winsparkle.h.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}
fn wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: host owns a readable, NUL-terminated UTF-16 string during this call.
    unsafe {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_init() {
    guard((), updater::init);
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_cleanup() {
    guard((), updater::cleanup);
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_lang(s: *const c_char) {
    guard((), || updater::configure(|c| c.lang = Some(narrow(s))));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_langid(id: u16) {
    guard((), || updater::configure(|c| c.langid = Some(id)));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_appcast_url(s: *const c_char) {
    guard((), || updater::configure(|c| c.feed = narrow(s)));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_registry_path(s: *const c_char) {
    guard((), || updater::configure(|c| c.registry = narrow(s)));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_app_details(
    company: *const u16,
    app: *const u16,
    version: *const u16,
) {
    guard((), || {
        updater::configure(|c| {
            c.company = wide(company);
            c.app = wide(app);
            c.version = wide(version);
        })
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_app_build_version(s: *const u16) {
    guard((), || updater::configure(|c| c.build = wide(s)));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_install_after_download(enabled: i32) {
    guard((), || updater::configure(|c| c.one_click = enabled != 0));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_theme(theme: i32) -> i32 {
    guard(0, || {
        let Some(theme) = updater::ThemeMode::from_abi(theme) else {
            return 0;
        };
        updater::set_theme(theme);
        1
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_eddsa_public_key(s: *const c_char) -> i32 {
    guard(0, || {
        let key = narrow(s);
        let valid = crate::crypto::valid_key(&key);
        // Preserve a failed key configuration as an error policy; never silently downgrade to unsigned.
        updater::configure(|c| c.key = Some(key));
        i32::from(valid)
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_dsa_pub_pem(_: *const c_char) -> i32 {
    guard(0, || {
        updater::configure(|c| c.dsa_required = true);
        crate::platform::log("DSA is unsupported; configure Ed25519 instead");
        0
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_http_header(name: *const c_char, value: *const c_char) {
    guard((), || {
        let name = narrow(name);
        let value = narrow(value);
        if crate::http::valid_header(&name, &value) {
            updater::configure(|c| {
                c.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
                c.headers.push((name, value));
            });
        }
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_clear_http_headers() {
    guard((), || updater::configure(|c| c.headers.clear()));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_config_methods(ptr: *const ConfigMethodsRaw) {
    guard((), || {
        if ptr.is_null() {
            updater::configure(|c| c.methods = None);
            return;
        }
        // SAFETY: host supplies a readable struct; copy it so its storage need not outlive the call.
        let raw = unsafe { *ptr };
        if let (Some(read), Some(write), Some(delete)) = (raw.read, raw.write, raw.delete) {
            updater::configure(|c| {
                c.methods = Some(ConfigMethods {
                    read,
                    write,
                    delete,
                    data: raw.data as usize,
                })
            });
        }
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_automatic_check_for_updates(state: i32) {
    guard((), || {
        settings::write(
            &updater::snapshot(),
            "CheckForUpdates",
            if state != 0 { "1" } else { "0" },
        )
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_get_automatic_check_for_updates() -> i32 {
    guard(0, || {
        i32::from(settings::number(&updater::snapshot(), "CheckForUpdates", 0) != 0)
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_set_update_check_interval(interval: i32) {
    guard((), || {
        settings::write(
            &updater::snapshot(),
            "UpdateInterval",
            &interval.max(3600).to_string(),
        )
    });
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_get_update_check_interval() -> i32 {
    guard(86400, || {
        settings::number(&updater::snapshot(), "UpdateInterval", 86400).clamp(3600, i32::MAX as i64)
            as i32
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_get_last_check_time() -> i64 {
    guard(-1, || {
        settings::number(&updater::snapshot(), "LastCheckTime", -1)
    })
}
macro_rules! callback {
    ($name:ident,$field:ident,$ty:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(callback: Option<$ty>) {
            guard((), || updater::callbacks(|c| c.$field = callback));
        }
    };
}
callback!(win_sparkle_set_error_callback, error, VoidCallback);
callback!(
    win_sparkle_set_can_shutdown_callback,
    can_shutdown,
    IntCallback
);
callback!(
    win_sparkle_set_shutdown_request_callback,
    shutdown,
    VoidCallback
);
callback!(
    win_sparkle_set_did_find_update_callback,
    found,
    VoidCallback
);
callback!(
    win_sparkle_set_did_not_find_update_callback,
    not_found,
    VoidCallback
);
callback!(
    win_sparkle_set_update_cancelled_callback,
    cancelled,
    VoidCallback
);
callback!(
    win_sparkle_set_update_skipped_callback,
    skipped,
    VoidCallback
);
callback!(
    win_sparkle_set_update_postponed_callback,
    postponed,
    VoidCallback
);
callback!(
    win_sparkle_set_update_dismissed_callback,
    dismissed,
    VoidCallback
);
callback!(
    win_sparkle_set_user_run_installer_callback,
    installer,
    InstallerCallback
);
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_check_update_with_ui() {
    guard((), || updater::check(CheckMode::Manual));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_check_update_without_ui() {
    guard((), || updater::check(CheckMode::Background));
}
#[unsafe(no_mangle)]
pub extern "C" fn win_sparkle_check_update_with_ui_and_install() {
    guard((), || updater::check(CheckMode::AutomaticInstall));
}
