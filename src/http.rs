//! A synchronous worker facade over cancellable native WinHTTP async operations.
use crate::platform::{error, wide};
use std::{
    collections::VecDeque,
    ffi::c_void,
    fs::OpenOptions,
    io::Write,
    path::Path,
    ptr::{null, null_mut},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, Networking::WinHttp::*};

#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> crate::Result<()> {
        if self.is_cancelled() {
            Err("Cancelled".into())
        } else {
            Ok(())
        }
    }
}
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this RAII guard exclusively owns its WinHTTP session/connection.
        unsafe {
            if !self.0.is_null() {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}
#[derive(Default)]
struct Completion {
    events: VecDeque<(u32, u32)>,
    closed: bool,
}
#[derive(Default)]
struct Context {
    state: Mutex<Completion>,
    changed: Condvar,
}
unsafe extern "system" fn status_callback(
    _: *mut c_void,
    context: usize,
    status: u32,
    info: *mut c_void,
    length: u32,
) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context == 0 {
            return;
        }
        let ptr = context as *const Context;
        // SAFETY: a registration Arc is retained until HANDLE_CLOSING; each callback borrows its own Arc.
        let context = unsafe {
            Arc::increment_strong_count(ptr);
            Arc::from_raw(ptr)
        };
        let mut state = context.state.lock().unwrap_or_else(|p| p.into_inner());
        match status {
            WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING => {
                state.closed = true;
                // SAFETY: this last notification releases exactly the registration's Arc.
                unsafe {
                    drop(Arc::from_raw(ptr));
                }
            }
            WINHTTP_CALLBACK_STATUS_REQUEST_ERROR => {
                // SAFETY: WinHTTP supplies a bounded WINHTTP_ASYNC_RESULT for this notification.
                let code = if !info.is_null()
                    && length as usize >= std::mem::size_of::<WINHTTP_ASYNC_RESULT>()
                {
                    unsafe { (*info.cast::<WINHTTP_ASYNC_RESULT>()).dwError }
                } else {
                    ERROR_WINHTTP_INTERNAL_ERROR
                };
                state.events.push_back((status, code));
            }
            WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE
            | WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE => state.events.push_back((status, 0)),
            WINHTTP_CALLBACK_STATUS_READ_COMPLETE => state.events.push_back((status, length)),
            _ => {}
        }
        context.changed.notify_all();
    }));
}
struct Request {
    handle: *mut c_void,
    context: Arc<Context>,
}
impl Request {
    fn new(handle: *mut c_void) -> crate::Result<Self> {
        if handle.is_null() {
            return Err(error("WinHttpOpenRequest"));
        }
        let context = Arc::new(Context::default());
        let ptr = Arc::into_raw(context.clone()) as usize;
        // SAFETY: context has a registration Arc; the request has not started any operation.
        unsafe {
            if WinHttpSetOption(
                handle,
                WINHTTP_OPTION_CONTEXT_VALUE,
                (&ptr as *const usize).cast(),
                std::mem::size_of::<usize>() as u32,
            ) == 0
            {
                drop(Arc::from_raw(ptr as *const Context));
                WinHttpCloseHandle(handle);
                return Err(error("WinHTTP request context"));
            }
            let previous = WinHttpSetStatusCallback(
                handle,
                Some(status_callback),
                WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS,
                0,
            );
            if previous.is_some_and(|f| f as usize == usize::MAX) {
                drop(Arc::from_raw(ptr as *const Context));
                WinHttpCloseHandle(handle);
                return Err(error("WinHTTP status callback"));
            }
        }
        Ok(Self { handle, context })
    }
    fn wait(&self, expected: u32, token: &CancellationToken) -> crate::Result<u32> {
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut state = self.context.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            token.check()?;
            while let Some((status, value)) = state.events.pop_front() {
                if status == WINHTTP_CALLBACK_STATUS_REQUEST_ERROR {
                    return Err(format!(
                        "HTTP: {}",
                        std::io::Error::from_raw_os_error(value as i32)
                    ));
                }
                if status == expected {
                    return Ok(value);
                }
            }
            if state.closed {
                return Err("HTTP request closed".into());
            }
            if Instant::now() >= deadline {
                return Err("HTTP operation timed out".into());
            }
            state = self
                .context
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        // SAFETY: only this worker calls request APIs; async close safely cancels pending I/O.
        unsafe {
            WinHttpCloseHandle(self.handle);
        }
        let mut state = self.context.state.lock().unwrap_or_else(|p| p.into_inner());
        while !state.closed {
            state = self
                .context
                .changed
                .wait(state)
                .unwrap_or_else(|p| p.into_inner());
        }
    }
}

pub struct HttpRequest<'a> {
    pub url: &'a str,
    pub headers: &'a [(String, String)],
    pub user_agent: &'a str,
}
pub struct HttpResponse {
    pub body: Vec<u8>,
    pub metadata: HttpResponseMetadata,
}
#[derive(Clone, Debug)]
pub struct HttpResponseMetadata {
    pub status: u32,
    pub content_length: Option<u64>,
}
pub trait HttpClient {
    fn get_bytes(
        &self,
        request: HttpRequest<'_>,
        cancellation: &CancellationToken,
    ) -> crate::Result<HttpResponse>;
    fn download(
        &self,
        request: HttpRequest<'_>,
        destination: &Path,
        cancellation: &CancellationToken,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> crate::Result<HttpResponseMetadata>;
}
pub struct WinHttp;
struct Url {
    host: Vec<u16>,
    path: Vec<u16>,
    origin: String,
    secure: bool,
    port: u16,
}
fn parse_url(url: &str) -> crate::Result<Url> {
    if url.contains(['\r', '\n', '\0']) {
        return Err("Invalid update URL".into());
    }
    let url = wide(url);
    // SAFETY: CrackUrl components reference the live UTF-16 buffer and are copied before return.
    unsafe {
        let mut parts: URL_COMPONENTS = std::mem::zeroed();
        parts.dwStructSize = std::mem::size_of_val(&parts) as u32;
        parts.dwHostNameLength = u32::MAX;
        parts.dwUrlPathLength = u32::MAX;
        parts.dwExtraInfoLength = u32::MAX;
        parts.dwUserNameLength = u32::MAX;
        parts.dwPasswordLength = u32::MAX;
        if WinHttpCrackUrl(url.as_ptr(), 0, 0, &mut parts) == 0 {
            return Err(error("Invalid update URL"));
        }
        if ![1, 2].contains(&parts.nScheme)
            || parts.dwHostNameLength == 0
            || parts.dwUserNameLength != 0
            || parts.dwPasswordLength != 0
        {
            return Err("Only HTTP(S) URLs without credentials are supported".into());
        }
        let host = std::slice::from_raw_parts(parts.lpszHostName, parts.dwHostNameLength as usize);
        let mut ascii = vec![0u16; 1024];
        let len = windows_sys::Win32::Globalization::IdnToAscii(
            0,
            host.as_ptr(),
            host.len() as i32,
            ascii.as_mut_ptr(),
            ascii.len() as i32,
        );
        let mut host = if len > 0 {
            ascii.truncate(len as usize);
            ascii
        } else if host.iter().all(|c| *c < 128) {
            host.to_vec()
        } else {
            return Err("Invalid international hostname".into());
        };
        let origin = format!(
            "{}://{}:{}",
            parts.nScheme,
            String::from_utf16_lossy(&host).to_ascii_lowercase(),
            parts.nPort
        );
        host.push(0);
        let mut path = if parts.dwUrlPathLength > 0 {
            std::slice::from_raw_parts(parts.lpszUrlPath, parts.dwUrlPathLength as usize).to_vec()
        } else {
            vec!['/' as u16]
        };
        if parts.dwExtraInfoLength > 0 {
            path.extend_from_slice(std::slice::from_raw_parts(
                parts.lpszExtraInfo,
                parts.dwExtraInfoLength as usize,
            ));
        }
        if let Some(i) = path.iter().position(|c| *c == '#' as u16) {
            path.truncate(i);
        }
        path.push(0);
        Ok(Url {
            host,
            path,
            origin,
            secure: parts.nScheme == 2,
            port: parts.nPort,
        })
    }
}
fn redirect_url(base: &str, location: &str) -> crate::Result<String> {
    let mut result = vec![0u16; 32768];
    let mut length = result.len() as u32;
    // SAFETY: UrlCombineW reads valid UTF-16 strings and writes within the bounded output buffer.
    if unsafe {
        windows_sys::Win32::UI::Shell::UrlCombineW(
            wide(base).as_ptr(),
            wide(location).as_ptr(),
            result.as_mut_ptr(),
            &mut length,
            0,
        )
    } < 0
    {
        return Err("Invalid HTTP redirect".into());
    }
    Ok(String::from_utf16_lossy(&result[..length as usize]))
}
fn header(handle: *mut c_void, query: u32) -> Option<String> {
    let mut length = 0;
    // SAFETY: query size first, then allocate a sufficiently large UTF-16 buffer.
    unsafe {
        WinHttpQueryHeaders(handle, query, null(), null_mut(), &mut length, null_mut());
        if length == 0 || length > 65536 {
            return None;
        }
        let mut text = vec![0u16; length as usize / 2 + 1];
        if WinHttpQueryHeaders(
            handle,
            query,
            null(),
            text.as_mut_ptr().cast(),
            &mut length,
            null_mut(),
        ) == 0
        {
            return None;
        }
        Some(String::from_utf16_lossy(
            &text[..text.iter().position(|c| *c == 0).unwrap_or(text.len())],
        ))
    }
}

impl WinHttp {
    fn transfer(
        &self,
        req: HttpRequest<'_>,
        token: &CancellationToken,
        mut sink: impl FnMut(&[u8], Option<u64>) -> crate::Result<()>,
    ) -> crate::Result<HttpResponseMetadata> {
        token.check()?;
        let original = parse_url(req.url)?;
        let mut was_secure = original.secure;
        let mut current = req.url.to_owned();
        // This buffer outlives all request guards, including pending-read cancellation and HANDLE_CLOSING.
        let mut bytes = [0u8; 65536];
        // SAFETY: session is explicitly async; worker waits for native completion notifications before reusing buffers.
        unsafe {
            let session = Handle(WinHttpOpen(
                wide(req.user_agent).as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                WINHTTP_FLAG_ASYNC,
            ));
            if session.0.is_null() {
                return Err(error("WinHttpOpen"));
            }
            if WinHttpSetTimeouts(session.0, 10000, 10000, 30000, 30000) == 0 {
                return Err(error("WinHTTP timeouts"));
            }
            let deadline = Instant::now() + Duration::from_secs(30 * 60);
            for _ in 0..=10 {
                token.check()?;
                let url = parse_url(&current)?;
                if was_secure && !url.secure {
                    return Err("HTTPS to HTTP redirect refused".into());
                }
                was_secure |= url.secure;
                let connection = Handle(WinHttpConnect(session.0, url.host.as_ptr(), url.port, 0));
                if connection.0.is_null() {
                    return Err(error("WinHttpConnect"));
                }
                let request = Request::new(WinHttpOpenRequest(
                    connection.0,
                    wide("GET").as_ptr(),
                    url.path.as_ptr(),
                    null(),
                    null(),
                    null(),
                    if url.secure { WINHTTP_FLAG_SECURE } else { 0 },
                ))?;
                let handle = request.handle;
                let disable = WINHTTP_DISABLE_REDIRECTS;
                if WinHttpSetOption(
                    handle,
                    WINHTTP_OPTION_DISABLE_FEATURE,
                    (&disable as *const u32).cast(),
                    4,
                ) == 0
                {
                    return Err(error("WinHTTP redirect policy"));
                }
                let mut headers = String::from("Cache-Control: no-cache\r\n");
                // Do not forward application secrets to another origin after redirect.
                if url.origin == original.origin {
                    for (name, value) in req.headers {
                        if !valid_header(name, value) {
                            return Err("Invalid custom HTTP header".into());
                        }
                        headers.push_str(&format!("{name}: {value}\r\n"));
                    }
                }
                let headers = wide(&headers);
                token.check()?;
                if WinHttpSendRequest(
                    handle,
                    headers.as_ptr(),
                    (headers.len() - 1) as u32,
                    null(),
                    0,
                    0,
                    Arc::as_ptr(&request.context) as usize,
                ) == 0
                    && GetLastError() != ERROR_IO_PENDING
                {
                    return Err(error("HTTP request"));
                }
                request.wait(WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE, token)?;
                token.check()?;
                if WinHttpReceiveResponse(handle, null_mut()) == 0
                    && GetLastError() != ERROR_IO_PENDING
                {
                    return Err(error("HTTP response"));
                }
                request.wait(WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE, token)?;
                let status: u32 = header(handle, WINHTTP_QUERY_STATUS_CODE)
                    .ok_or("Missing HTTP status")?
                    .parse()
                    .map_err(|_| "Invalid HTTP status")?;
                if [301, 302, 303, 307, 308].contains(&status) {
                    current = redirect_url(
                        &current,
                        &header(handle, WINHTTP_QUERY_LOCATION)
                            .ok_or("Missing HTTP redirect location")?,
                    )?;
                    continue;
                }
                if !(200..300).contains(&status) {
                    return Err(format!("HTTP {status}"));
                }
                let total = header(handle, WINHTTP_QUERY_CONTENT_LENGTH)
                    .and_then(|s| s.parse::<u64>().ok());
                let mut received = 0u64;
                loop {
                    token.check()?;
                    if Instant::now() > deadline {
                        return Err("Download timed out".into());
                    }
                    if WinHttpReadData(
                        handle,
                        bytes.as_mut_ptr().cast(),
                        bytes.len() as u32,
                        null_mut(),
                    ) == 0
                        && GetLastError() != ERROR_IO_PENDING
                    {
                        return Err(error("HTTP read"));
                    }
                    let read = request.wait(WINHTTP_CALLBACK_STATUS_READ_COMPLETE, token)?;
                    if read == 0 {
                        break;
                    }
                    if read as usize > bytes.len() {
                        return Err("Invalid HTTP read length".into());
                    }
                    sink(&bytes[..read as usize], total)?;
                    received = received
                        .checked_add(read as u64)
                        .ok_or("HTTP response length overflow")?;
                }
                if total.is_some_and(|length| length != received) {
                    return Err("Incomplete HTTP response (Content-Length mismatch)".into());
                }
                token.check()?;
                return Ok(HttpResponseMetadata {
                    status,
                    content_length: total,
                });
            }
            Err("Too many HTTP redirects".into())
        }
    }
}
pub fn valid_header(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        && !value.contains(['\r', '\n', '\0'])
}
impl HttpClient for WinHttp {
    fn get_bytes(
        &self,
        request: HttpRequest<'_>,
        token: &CancellationToken,
    ) -> crate::Result<HttpResponse> {
        let mut body = Vec::new();
        let metadata = self.transfer(request, token, |bytes, _| {
            if body.len() + bytes.len() > 4 * 1024 * 1024 {
                return Err("Appcast exceeds 4 MiB".into());
            }
            body.extend_from_slice(bytes);
            Ok(())
        })?;
        Ok(HttpResponse { body, metadata })
    }
    fn download(
        &self,
        request: HttpRequest<'_>,
        destination: &Path,
        token: &CancellationToken,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> crate::Result<HttpResponseMetadata> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|e| e.to_string())?;
        let mut received = 0u64;
        let mut last = Instant::now() - Duration::from_secs(1);
        let result = self.transfer(request, token, |bytes, total| {
            file.write_all(bytes).map_err(|e| e.to_string())?;
            received = received
                .checked_add(bytes.len() as u64)
                .ok_or("Download size overflow")?;
            if last.elapsed() >= Duration::from_millis(100) {
                progress(received, total);
                last = Instant::now();
            }
            Ok(())
        });
        let result = result.and_then(|meta| {
            file.sync_all().map_err(|e| e.to_string())?;
            progress(received, meta.content_length);
            Ok(meta)
        });
        drop(file);
        if result.is_err() {
            let _ = std::fs::remove_file(destination);
        }
        result
    }
}
