use crate::platform::{path_wide, wide};
use std::{
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL};

pub fn launch(
    path: &Path,
    arguments: &str,
    callback: Option<crate::ffi::InstallerCallback>,
) -> crate::Result<()> {
    let path = path_wide(path);
    let arguments = wide(arguments);
    if let Some(callback) = callback {
        // SAFETY: callback belongs to the host and the path buffer lives for the entire call.
        match unsafe { callback(path.as_ptr()) } {
            0 => {}
            1 => return Ok(()),
            _ => return Err("Custom installer callback failed".into()),
        }
    }
    // SAFETY: all fields are initialized, ShellExecuteEx copies parameters before returning.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        info.fMask = SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS;
        info.lpFile = path.as_ptr();
        info.lpParameters = arguments.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) == 0 {
            return Err(crate::platform::error("Failed to launch installer"));
        }
        if !info.hProcess.is_null() {
            drop(crate::platform::KernelHandle(info.hProcess));
        }
    }
    Ok(())
}
pub fn open_url(url: &str) -> crate::Result<()> {
    if !crate::appcast::http_url(url) {
        return Err("Invalid browser URL".into());
    }
    // SAFETY: URL is restricted to HTTP(S) and passed as lpFile, never as shell command text.
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(crate::platform::error("Cannot open browser"))
    } else {
        Ok(())
    }
}
