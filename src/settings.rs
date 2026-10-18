use crate::{platform::wide, updater::Config};
use std::{
    ffi::CString,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::ERROR_SUCCESS, System::Registry::*};

pub fn read(config: &Config, name: &str) -> Option<String> {
    if let Some(methods) = config.methods {
        let name = CString::new(name).ok()?;
        let mut buffer = vec![0u16; 32768];
        // SAFETY: methods are copied from the C ABI; host retains callbacks/user data until cleanup.
        if unsafe {
            (methods.read)(
                name.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                methods.data as *mut _,
            )
        } == 0
        {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0)?;
        return String::from_utf16(&buffer[..end]).ok();
    }
    let path = wide(&config.registry_path());
    let name = wide(name);
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut size = 0;
        // SAFETY: Win32 first queries size, then writes to a suitably sized UTF-16 buffer.
        unsafe {
            if RegGetValueW(
                root,
                path.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut size,
            ) != ERROR_SUCCESS
                || size > 65536
            {
                continue;
            }
            let mut value = vec![0u16; size as usize / 2 + 1];
            if RegGetValueW(
                root,
                path.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                value.as_mut_ptr().cast(),
                &mut size,
            ) == ERROR_SUCCESS
            {
                return String::from_utf16(
                    &value[..value.iter().position(|c| *c == 0).unwrap_or(value.len())],
                )
                .ok();
            }
        }
    }
    None
}
pub fn write(config: &Config, name: &str, value: &str) {
    let cname = CString::new(name).expect("static settings key");
    let value = wide(value);
    if let Some(methods) = config.methods {
        // SAFETY: host callback contract, buffers stay alive for this synchronous call; no mutex held.
        unsafe {
            (methods.write)(cname.as_ptr(), value.as_ptr(), methods.data as *mut _);
        }
        return;
    }
    let path = wide(&config.registry_path());
    let name = wide(name);
    // SAFETY: opened registry key is closed exactly once; all buffer sizes are in bytes.
    unsafe {
        let mut key = null_mut();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            null(),
            0,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        ) == ERROR_SUCCESS
        {
            let result = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            );
            RegCloseKey(key);
            if result != ERROR_SUCCESS {
                crate::platform::log("Cannot write registry settings");
            }
        }
    }
}
pub fn delete(config: &Config, name: &str) {
    if let Some(methods) = config.methods {
        let name = CString::new(name).expect("static settings key");
        // SAFETY: copied callback; no mutex held across host code.
        unsafe {
            (methods.delete)(name.as_ptr(), methods.data as *mut _);
        }
        return;
    }
    // SAFETY: registry API reads the supplied NUL-terminated strings synchronously.
    unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            wide(&config.registry_path()).as_ptr(),
            wide(name).as_ptr(),
        );
    }
}
pub fn number(config: &Config, name: &str, default: i64) -> i64 {
    read(config, name)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
