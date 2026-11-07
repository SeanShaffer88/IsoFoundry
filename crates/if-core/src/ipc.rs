//! Protocol between `isofoundry.exe` (unelevated) and
//! `isofoundry-helper.exe` (elevated on demand). D-014.
//!
//! Frames are a 4-byte little-endian length followed by UTF-8 JSON, at most
//! [`MAX_FRAME`] bytes. The helper must open with [`Request::Hello`] carrying
//! the per-session nonce the app passed on its command line; any other first
//! message, a wrong nonce or a protocol mismatch ends the session. Only typed
//! messages exist: no paths to executables, no shell strings.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};

pub const PROTOCOL: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
pub const NONCE_BYTES: usize = 32;

/// Messages the app sends to the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Version,
    Shutdown,
}

/// The helper's first message, proving it was started by this app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub nonce: String,
    pub helper_version: String,
    pub elevated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Pong,
    Version { version: String },
    Bye,
    Error { code: u8, message: String },
}

#[derive(Debug)]
pub enum FrameError {
    Io(std::io::Error),
    TooLarge(usize),
    Json(String),
    Closed,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Io(e) => write!(f, "pipe I/O: {e}"),
            FrameError::TooLarge(n) => write!(f, "frame of {n} bytes exceeds {MAX_FRAME}"),
            FrameError::Json(e) => write!(f, "malformed message: {e}"),
            FrameError::Closed => write!(f, "pipe closed"),
        }
    }
}

impl std::error::Error for FrameError {}

pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), FrameError> {
    let body = serde_json::to_vec(msg).map_err(|e| FrameError::Json(e.to_string()))?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge(body.len()));
    }
    w.write_all(&(body.len() as u32).to_le_bytes()).map_err(FrameError::Io)?;
    w.write_all(&body).map_err(FrameError::Io)?;
    w.flush().map_err(FrameError::Io)
}

pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> Result<T, FrameError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(FrameError::Closed),
        Err(e) => return Err(FrameError::Io(e)),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(FrameError::TooLarge(n));
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).map_err(FrameError::Io)?;
    serde_json::from_slice(&body).map_err(|e| FrameError::Json(e.to_string()))
}

/// 256-bit random nonce, hex encoded.
pub fn new_nonce() -> String {
    let mut b = [0u8; NONCE_BYTES];
    getrandom::fill(&mut b).expect("operating system random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Constant-time comparison so timing doesn't reveal the nonce.
pub fn nonce_matches(expected: &str, got: &str) -> bool {
    let (a, b) = (expected.as_bytes(), got.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Validates the helper's opening message.
pub fn check_hello(h: &Hello, expected_nonce: &str) -> Result<(), String> {
    if h.protocol != PROTOCOL {
        return Err(format!("helper speaks protocol {}, app speaks {PROTOCOL}", h.protocol));
    }
    if !nonce_matches(expected_nonce, &h.nonce) {
        return Err("helper presented the wrong session nonce".into());
    }
    if h.helper_version != crate::APP_VERSION {
        return Err(format!("helper version {} does not match app version {}", h.helper_version, crate::APP_VERSION));
    }
    Ok(())
}

/// Pipe name unique to this app process and session.
pub fn pipe_name(nonce: &str) -> String {
    format!(r"\\.\pipe\IsoFoundry-{}-{}", std::process::id(), &nonce[..16.min(nonce.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Request::Ping).unwrap();
        write_frame(&mut buf, &Response::Version { version: "x".into() }).unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame::<_, Request>(&mut r).unwrap(), Request::Ping);
        assert_eq!(read_frame::<_, Response>(&mut r).unwrap(), Response::Version { version: "x".into() });
        assert!(matches!(read_frame::<_, Request>(&mut r), Err(FrameError::Closed)));
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let mut buf = ((MAX_FRAME + 1) as u32).to_le_bytes().to_vec();
        buf.extend_from_slice(b"{}");
        assert!(matches!(read_frame::<_, Request>(&mut buf.as_slice()), Err(FrameError::TooLarge(_))));
    }

    #[test]
    fn unknown_message_is_rejected() {
        let body = br#"{"type":"run_command","cmd":"calc.exe"}"#;
        let mut buf = (body.len() as u32).to_le_bytes().to_vec();
        buf.extend_from_slice(body);
        assert!(matches!(read_frame::<_, Request>(&mut buf.as_slice()), Err(FrameError::Json(_))));
    }

    #[test]
    fn nonce_checks() {
        let n = new_nonce();
        assert_eq!(n.len(), NONCE_BYTES * 2);
        assert_ne!(n, new_nonce());
        let good =
            Hello { protocol: PROTOCOL, nonce: n.clone(), helper_version: crate::APP_VERSION.into(), elevated: true };
        assert!(check_hello(&good, &n).is_ok());
        let bad = Hello { nonce: "0".repeat(64), ..good.clone() };
        assert!(check_hello(&bad, &n).is_err());
        let old = Hello { protocol: 0, ..good.clone() };
        assert!(check_hello(&old, &n).is_err());
        assert!(!nonce_matches("ab", "abc"));
    }

    #[test]
    fn pipe_name_shape() {
        let p = pipe_name(&"f".repeat(64));
        assert!(p.starts_with(r"\\.\pipe\IsoFoundry-"));
        assert!(p.ends_with(&"f".repeat(16)));
    }
}
