use std::ffi::{CStr, c_char};

// Both variants download into a bounded memory sink, with the same release profile.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn spike_get(url: *const c_char) -> i64 {
    std::panic::catch_unwind(|| {
        if url.is_null() {
            return -1;
        }
        // SAFETY: the spike caller supplies a NUL-terminated UTF-8 URL.
        let url = unsafe { CStr::from_ptr(url) }.to_string_lossy();
        get(&url).unwrap_or(-1)
    })
    .unwrap_or(-1)
}

#[cfg(feature = "curl-backend")]
fn get(url: &str) -> Option<i64> {
    let mut easy = curl::easy::Easy::new();
    easy.url(url).ok()?;
    easy.follow_location(true).ok()?;
    easy.timeout(std::time::Duration::from_secs(30)).ok()?;
    let mut size = 0i64;
    {
        let mut transfer = easy.transfer();
        transfer
            .write_function(|bytes| {
                size += bytes.len() as i64;
                Ok(bytes.len())
            })
            .ok()?;
        transfer.perform().ok()?;
    }
    (easy.response_code().ok()? == 200).then_some(size)
}

#[cfg(not(feature = "curl-backend"))]
fn get(url: &str) -> Option<i64> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Networking::WinHttp::*;
    let wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
    let agent: Vec<u16> = "WinSparkle-spike".encode_utf16().chain([0]).collect();
    // SAFETY: all buffers live for the synchronous WinHTTP calls, handles are closed below.
    unsafe {
        let mut parts: URL_COMPONENTS = std::mem::zeroed();
        parts.dwStructSize = std::mem::size_of_val(&parts) as u32;
        parts.dwHostNameLength = u32::MAX;
        parts.dwUrlPathLength = u32::MAX;
        parts.dwExtraInfoLength = u32::MAX;
        if WinHttpCrackUrl(wide.as_ptr(), 0, 0, &mut parts) == 0 {
            return None;
        }
        let host: Vec<_> =
            std::slice::from_raw_parts(parts.lpszHostName, parts.dwHostNameLength as usize)
                .iter()
                .copied()
                .chain([0])
                .collect();
        let path: Vec<_> = std::slice::from_raw_parts(
            parts.lpszUrlPath,
            (parts.dwUrlPathLength + parts.dwExtraInfoLength) as usize,
        )
        .iter()
        .copied()
        .chain([0])
        .collect();
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        );
        if session.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session, 10000, 10000, 30000, 30000);
        let connection = WinHttpConnect(session, host.as_ptr(), parts.nPort, 0);
        let flags = if parts.nScheme == 2 {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        let request = WinHttpOpenRequest(
            connection,
            null(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            flags,
        );
        let mut result = None;
        if !request.is_null()
            && WinHttpSendRequest(request, null(), 0, null(), 0, 0, 0) != 0
            && WinHttpReceiveResponse(request, null_mut()) != 0
        {
            let mut size = 0;
            let mut buffer = [0u8; 65536];
            loop {
                let mut read = 0;
                if WinHttpReadData(
                    request,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                ) == 0
                {
                    break;
                }
                if read == 0 {
                    result = Some(size);
                    break;
                }
                size += read as i64;
            }
        }
        if !request.is_null() {
            WinHttpCloseHandle(request);
        }
        if !connection.is_null() {
            WinHttpCloseHandle(connection);
        }
        WinHttpCloseHandle(session);
        result
    }
}
