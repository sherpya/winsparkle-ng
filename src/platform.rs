use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::*, System::LibraryLoader::*};

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
pub fn window_long(window: HWND, index: i32) -> isize {
    // SAFETY: Win32 validates the HWND; normalize the x86 LONG and x64 LONG_PTR bindings.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(window, index) as isize
    }
}
pub fn set_window_long(window: HWND, index: i32, value: isize) -> isize {
    // SAFETY: Win32 validates the HWND/index; x86 isize is exactly the width of a LONG.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(window, index, value as _)
            as isize
    }
}
pub fn path_wide(value: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_os_str().encode_wide().chain([0]).collect()
}
pub fn error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}
pub fn log(message: &str) {
    // SAFETY: OutputDebugStringW reads the NUL-terminated temporary synchronously.
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW(
            wide(&format!("WinSparkle-NG: {message}\n")).as_ptr(),
        );
    }
}
pub fn module() -> HMODULE {
    let mut result = null_mut();
    // SAFETY: FROM_ADDRESS treats this pointer as an address inside this DLL, not as text.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            module as *const () as *const u16,
            &mut result,
        );
    }
    result
}
pub fn resource(
    module: HMODULE,
    name: *const u16,
    kind: *const u16,
    lang: Option<u16>,
) -> Option<&'static [u8]> {
    // SAFETY: resource memory is read-only and lives while the host/DLL is loaded.
    unsafe {
        let res = if let Some(lang) = lang {
            FindResourceExW(module, kind, name, lang)
        } else {
            FindResourceW(module, name, kind)
        };
        if res.is_null() {
            return None;
        }
        let size = SizeofResource(module, res) as usize;
        let loaded = LoadResource(module, res);
        let ptr = LockResource(loaded);
        if ptr.is_null() {
            None
        } else {
            Some(std::slice::from_raw_parts(ptr.cast(), size))
        }
    }
}
pub fn host_resource(name: &str, kind: &str) -> Option<String> {
    // SAFETY: a null module name requests the already-loaded host EXE.
    let host = unsafe { GetModuleHandleW(null()) };
    resource(host, wide(name).as_ptr(), wide(kind).as_ptr(), None)
        .and_then(|s| std::str::from_utf8(s).ok())
        .map(|s| s.trim_matches('\0').trim().to_owned())
}

pub struct TempDir {
    pub path: PathBuf,
    pub keep: bool,
}
impl TempDir {
    pub fn create() -> crate::Result<Self> {
        use windows_sys::Win32::Security::Cryptography::*;
        for _ in 0..16 {
            let mut random = [0u8; 16];
            // SAFETY: BCrypt writes precisely the supplied buffer; no algorithm handle is used.
            if unsafe {
                BCryptGenRandom(
                    null_mut(),
                    random.as_mut_ptr(),
                    16,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            } < 0
            {
                return Err("Cannot generate temporary directory name".into());
            }
            let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let path = std::env::temp_dir().join(format!("WinSparkle-NG-{token}"));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, keep: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Cannot create private temporary directory".into())
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        if !self.keep {
            clean_temp(&self.path);
        }
    }
}
pub fn clean_temp(path: &Path) {
    // Only our direct children of the actual temp directory are eligible; never follow a junction.
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return;
    };
    let suffix = name.strip_prefix("WinSparkle-NG-").unwrap_or("");
    if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return;
    }
    if path.parent() != Some(std::env::temp_dir().as_path()) {
        return;
    }
    use std::os::windows::fs::MetadataExt;
    let Ok(meta) = path.symlink_metadata() else {
        return;
    };
    if meta.file_attributes() & 0x400 != 0 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(path);
}

pub struct KernelHandle(pub HANDLE);
impl Drop for KernelHandle {
    fn drop(&mut self) {
        // SAFETY: this RAII type exclusively owns a valid kernel handle.
        unsafe {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                CloseHandle(self.0);
            }
        }
    }
}
pub fn raw(pointer: usize) -> *mut c_void {
    pointer as *mut c_void
}

pub fn host_metadata() -> (String, String, String) {
    use windows_sys::Win32::Storage::FileSystem::*;
    // SAFETY: version APIs return pointers into the owned version buffer, consumed before it drops.
    unsafe {
        let mut path = vec![0u16; 32768];
        let count = GetModuleFileNameW(null_mut(), path.as_mut_ptr(), path.len() as u32);
        if count == 0 || count as usize >= path.len() {
            return Default::default();
        }
        let size = GetFileVersionInfoSizeW(path.as_ptr(), null_mut());
        if size == 0 {
            return Default::default();
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return Default::default();
        }
        let mut translation = null_mut();
        let mut len = 0;
        let (lang, page) = if VerQueryValueW(
            data.as_ptr().cast(),
            wide("\\VarFileInfo\\Translation").as_ptr(),
            &mut translation,
            &mut len,
        ) != 0
            && len >= 4
        {
            let words = std::slice::from_raw_parts(translation.cast::<u16>(), 2);
            (words[0], words[1])
        } else {
            (0x409, 1200)
        };
        let field = |name: &str| {
            let query = wide(&format!("\\StringFileInfo\\{lang:04x}{page:04x}\\{name}"));
            let mut ptr = null_mut();
            let mut len = 0;
            if VerQueryValueW(data.as_ptr().cast(), query.as_ptr(), &mut ptr, &mut len) != 0
                && len > 0
                && !ptr.is_null()
            {
                let words = std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize);
                String::from_utf16_lossy(
                    &words[..words.iter().position(|c| *c == 0).unwrap_or(words.len())],
                )
            } else {
                String::new()
            }
        };
        (
            field("CompanyName"),
            field("ProductName"),
            field("ProductVersion"),
        )
    }
}
pub fn supports_os(min: &str) -> bool {
    if min.is_empty() {
        return true;
    }
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOEXW;
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OSVERSIONINFOEXW) -> i32;
    }
    // SAFETY: RtlGetVersion fills the correctly sized, initialized version structure.
    unsafe {
        let mut info: OSVERSIONINFOEXW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of_val(&info) as u32;
        if RtlGetVersion(&mut info) < 0 {
            return false;
        }
        os_meets_minimum(
            min,
            (info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber),
            info.wServicePackMajor,
        )
    }
}
fn os_meets_minimum(min: &str, actual: (u32, u32, u32), service_pack: u16) -> bool {
    let parts: Vec<u32> = min
        .split(['.', '-'])
        .map_while(|s| s.parse().ok())
        .collect();
    if parts.is_empty() {
        return true;
    }
    let major = parts[0];
    let minor = parts.get(1).copied().unwrap_or(0);
    let build = parts.get(2).copied().unwrap_or(0);
    // Preserve old major.minor.servicepack appcasts as well as modern major.minor-build.
    if build < 100 {
        (actual.0, actual.1) > (major, minor)
            || ((actual.0, actual.1) == (major, minor) && u32::from(service_pack) >= build)
    } else {
        actual >= (major, minor, build)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn minimum_windows_build_and_legacy_service_pack() {
        assert!(!super::os_meets_minimum("6.1.1", (6, 1, 7600), 0));
        assert!(super::os_meets_minimum("6.1.1", (6, 1, 7601), 1));
        assert!(super::os_meets_minimum("6.1.1", (10, 0, 19045), 0));
        assert!(!super::os_meets_minimum("10.0-22000", (10, 0, 19045), 0));
        assert!(super::os_meets_minimum("10.0.22000", (10, 0, 22631), 0));
    }
}
