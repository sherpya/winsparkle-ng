use WinSparkle::http::{CancellationToken, HttpClient, HttpRequest, WinHttp};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(handler: impl Fn(String, &mut TcpStream) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut request = String::new();
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                                break;
                            }
                            if line == "\r\n" {
                                break;
                            }
                            request.push_str(&line);
                        }
                        handler(request, &mut stream);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn get(url: &str) -> Result<Vec<u8>, String> {
    WinHttp
        .get_bytes(
            HttpRequest {
                url,
                headers: &[],
                user_agent: "WinSparkle-NG tests",
            },
            &CancellationToken::default(),
        )
        .map(|r| r.body)
}
#[test]
fn redirects_strip_custom_headers_on_origin_change() {
    let captured = Arc::new(Mutex::new(String::new()));
    let capture = captured.clone();
    let destination = Server::new(move |request, stream| {
        *capture.lock().unwrap() = request;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .unwrap();
    });
    let target = destination.url.clone();
    let source = Server::new(move |_, stream| {
        stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    });
    let response = WinHttp
        .get_bytes(
            HttpRequest {
                url: &source.url,
                headers: &[
                    ("Authorization".into(), "Bearer private".into()),
                    ("X-Private".into(), "secret".into()),
                ],
                user_agent: "test",
            },
            &CancellationToken::default(),
        )
        .unwrap();
    assert_eq!(response.body, b"OK");
    let received = captured.lock().unwrap().to_ascii_lowercase();
    assert!(!received.contains("private"));
    assert!(!received.contains("secret"));
}
#[test]
fn missing_length_and_truncated_body() {
    let server = Server::new(|request, stream| {
        let response = if request.starts_with("GET /truncated ") {
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort"
        } else {
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ncomplete"
        };
        stream.write_all(response.as_bytes()).unwrap();
    });
    assert_eq!(get(&server.url).unwrap(), b"complete");
    assert!(get(&format!("{}/truncated", server.url)).is_err());
}
#[test]
fn cancellation_interrupts_pending_response_and_cleans_partial_file() {
    let (tx, rx) = std::sync::mpsc::channel();
    let server = Server::new(move |_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\npartial");
        let _ = tx.send(());
        thread::sleep(Duration::from_secs(2));
    });
    let token = CancellationToken::default();
    let cancel = token.clone();
    let url = server.url.clone();
    let path =
        std::env::temp_dir().join(format!("winsparkle-cancel-test-{}.tmp", std::process::id()));
    let destination = path.clone();
    let worker = thread::spawn(move || {
        WinHttp.download(
            HttpRequest {
                url: &url,
                headers: &[],
                user_agent: "test",
            },
            &destination,
            &token,
            &|_, _| {},
        )
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let start = Instant::now();
    cancel.cancel();
    assert!(worker.join().unwrap().is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(!path.exists());
}
#[test]
fn redirect_loops_and_large_appcasts_are_bounded() {
    let server = Server::new(|request, stream| {
        if request.starts_with("GET /large ") {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(&vec![b'x'; 5 * 1024 * 1024]);
        } else {
            let _=stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    });
    assert!(get(&server.url).unwrap_err().contains("redirect"));
    assert!(
        get(&format!("{}/large", server.url))
            .unwrap_err()
            .contains("4 MiB")
    );
}

#[test]
#[ignore = "Exercises the actual 30-second WinHTTP receive timeout"]
fn native_read_timeout() {
    let release = Arc::new(AtomicBool::new(false));
    let released = release.clone();
    let server = Server::new(move |_, stream| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\npartial");
        let deadline = Instant::now() + Duration::from_secs(50);
        while !released.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
    });
    let start = Instant::now();
    let result = get(&server.url);
    release.store(true, Ordering::Release);
    assert!(result.is_err());
    assert!(start.elapsed() >= Duration::from_secs(20));
    assert!(start.elapsed() < Duration::from_secs(46));
}
