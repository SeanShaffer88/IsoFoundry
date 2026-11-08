//! Minimal HTTPS GET over WinHTTP, for the two opt-in online
//! features (update check, MusicBrainz lookup). Nothing in the app
//! calls this unless the user turned the feature on or ran its
//! command; there are no cookies and no background traffic.

use super::util::wide;
use std::ffi::c_void;
use windows_sys::Win32::Networking::WinHttp::*;

type H = *mut c_void;

fn close(h: H) {
    if !h.is_null() {
        // SAFETY: a handle we opened, closed at most once.
        unsafe { WinHttpCloseHandle(h) };
    }
}

/// GETs an `https://` URL and returns the body as text. Errors name
/// the step that failed; HTTP status != 200 is an error.
pub(crate) fn get(url: &str, user_agent: &str) -> Result<String, String> {
    let rest = url.strip_prefix("https://").ok_or("only https URLs are supported")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if host.is_empty() {
        return Err("the URL has no host".into());
    }
    let ua = wide(user_agent);
    let hostw = wide(host);
    let verb = wide("GET");
    let pathw = wide(path);
    unsafe {
        let session =
            WinHttpOpen(ua.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0);
        if session.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        let result = (|| {
            let connect = WinHttpConnect(session, hostw.as_ptr(), 443, 0);
            if connect.is_null() {
                return Err("WinHttpConnect failed".into());
            }
            let out = (|| {
                let request = WinHttpOpenRequest(
                    connect,
                    verb.as_ptr(),
                    pathw.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    WINHTTP_FLAG_SECURE,
                );
                if request.is_null() {
                    return Err("WinHttpOpenRequest failed".into());
                }
                let out = (|| {
                    if WinHttpSendRequest(request, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) == 0 {
                        return Err("sending the request failed".into());
                    }
                    if WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
                        return Err("receiving the response failed".into());
                    }
                    let mut status: u32 = 0;
                    let mut len = std::mem::size_of::<u32>() as u32;
                    WinHttpQueryHeaders(
                        request,
                        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                        std::ptr::null(),
                        &mut status as *mut u32 as *mut c_void,
                        &mut len,
                        std::ptr::null_mut(),
                    );
                    if status != 200 {
                        return Err(format!("the server answered HTTP {status}"));
                    }
                    let mut body = Vec::new();
                    let mut buf = [0u8; 16 * 1024];
                    loop {
                        let mut read: u32 = 0;
                        if WinHttpReadData(request, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut read) == 0 {
                            return Err("reading the response failed".into());
                        }
                        if read == 0 {
                            break;
                        }
                        body.extend_from_slice(&buf[..read as usize]);
                        if body.len() > 8 * 1024 * 1024 {
                            return Err("the response is unexpectedly large".into());
                        }
                    }
                    String::from_utf8(body).map_err(|_| "the response was not UTF-8 text".to_string())
                })();
                close(request);
                out
            })();
            close(connect);
            out
        })();
        close(session);
        result
    }
}
