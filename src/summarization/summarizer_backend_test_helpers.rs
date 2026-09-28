// Test helpers for the summarizer backend (issue #772, split from
// summarizer_backend.rs to keep the source file within the 500-line budget).
//
// Included via `#[path]` in `summarizer_backend.rs`; the functions are
// re-exported as `pub` from the parent module so existing tests in
// `apfel_server_tests.rs` and `summarizer_fullpath_tests.rs` keep resolving
// them through `super::summarizer_backend::{...}`.

use std::io::Read;
use std::net::SocketAddr;
use std::thread;

// --- Test helpers (used by apfel_server_tests.rs) ---

/// Spawn a detached HTTP listener on 127.0.0.1:0 that answers every request
/// with `status_code` and `body` (JSON). Returns the bound address; the
/// listener runs until the process exits. Stands up fake apfel / non-apfel /
/// 429 endpoints without a real apfel binary.
#[cfg(test)]
pub fn start_fake_http_server(status_code: u16, body: &str) -> std::io::Result<SocketAddr> {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let body = body.to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Drain the request (headers + any body) before replying.
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 {status_code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    Ok(addr)
}

/// Like `start_fake_http_server` but answers `flaky_first` times with
/// `first_status`/`first_body`, then forever with `then_status`/`then_body`.
/// Models the 429 → 200 recovery the bounded retry must survive.
#[cfg(test)]
pub fn start_flaky_http_server(
    flaky_first: u32,
    first_status: u16,
    first_body: &str,
    then_status: u16,
    then_body: &str,
) -> std::io::Result<SocketAddr> {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let first_body = first_body.to_string();
    let then_body = then_body.to_string();
    thread::spawn(move || {
        let mut count = 0u32;
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let (status, body) = if count < flaky_first {
                count += 1;
                (first_status, first_body.clone())
            } else {
                (then_status, then_body.clone())
            };
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });
    Ok(addr)
}
