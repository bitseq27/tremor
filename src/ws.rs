//! Minimal WebSocket client (RFC 6455) over tokio-rustls - just enough of the
//! protocol to speak the EMSC-CSEM realtime feed on
//! `wss://cobra.emsc-csem.org/seismologist` (the very socket the public
//! emsc-csem.org event pages are driven by).
//!
//! Deliberately tiny: masked client frames out, unmasked server frames in,
//! ping/pong + close handled, continuation fragments reassembled, JSON in and
//! out. No new third-party websocket crate is pulled in so the mingw
//! cross-build keeps its exact TLS stack (rustls + ring, already present via
//! reqwest).

use std::io;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_native_tls::TlsConnector;
use tokio_native_tls::TlsStream;

/// One received websocket message.
#[derive(Debug, Clone, PartialEq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    /// server sent a close frame
    Closed,
}

enum Stream {
    Tls(TlsStream<TcpStream>),
    Plain(TcpStream),
}

impl Stream {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tls(s) => s.read(buf).await,
            Stream::Plain(s) => s.read(buf).await,
        }
    }
    async fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        match self {
            Stream::Tls(s) => s.write_all(buf).await,
            Stream::Plain(s) => s.write_all(buf).await,
        }
    }
    async fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tls(s) => s.flush().await,
            Stream::Plain(s) => s.flush().await,
        }
    }
}

pub struct WsClient {
    stream: Stream,
    rx: Vec<u8>,
}

/// System TLS (schannel on Windows, Secure Transport on macOS, OpenSSL
/// elsewhere): the EMSC gateway answers queries only for system/browser-class
/// TLS clients, and shipping on the platform store avoids sidecar CAs.
fn tls_connector() -> io::Result<TlsConnector> {
    tokio_native_tls::native_tls::TlsConnector::new()
        .map(TlsConnector::from)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
}

/// base64 (standard alphabet) of a 16-byte nonce, for Sec-WebSocket-Key.
fn b64_nonce(n: &[u8; 16]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(24);
    for c in n.chunks(3) {
        let b = [c[0], c.get(1).copied().unwrap_or(0), c.get(2).copied().unwrap_or(0)];
        out.push(A[(b[0] >> 2) as usize] as char);
        out.push(A[(((b[0] & 3) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if c.len() > 1 {
            A[(((b[1] & 15) << 2) | (b[2] >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[(b[2] & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Cheap non-cryptographic nonce source (server-challenge irrelevant here).
fn nonce_bytes() -> [u8; 16] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut s = t ^ 0x2545_F491_4F6C_DD1D;
    let mut n = [0u8; 16];
    for b in n.iter_mut() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s & 0xff) as u8;
    }
    n
}

impl WsClient {
    /// Plain-TCP connect + upgrade handshake (local protocol conformance tests).
    pub async fn connect_plain(host: &str, port: u16, path: &str, origin: &str) -> io::Result<Self> {
        let tcp = TcpStream::connect((host, port)).await?;
        Self::handshake(Stream::Plain(tcp), host, path, origin).await
    }

    /// TLS connect + HTTP/1.1 upgrade handshake.
    pub async fn connect(host: &str, path: &str, origin: &str) -> io::Result<Self> {
        let tcp = TcpStream::connect((host, 443)).await?;
        let connector = tls_connector()?;
        let stream = connector
            .connect(host, tcp)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;
        Self::handshake(Stream::Tls(stream), host, path, origin).await
    }

    async fn handshake(mut stream: Stream, host: &str, path: &str, origin: &str) -> io::Result<Self> {

        let key = b64_nonce(&nonce_bytes());
        // the cobra gateway serves browser clients; a non-browser UA gets the
        // handshake but is then silently ignored, so identify as one.
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: Mozilla/5.0\r\nOrigin: {origin}\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await?;
        stream.flush().await?;

        let mut head = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];
        loop {
            if head.len() > 16 * 1024 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "handshake too big"));
            }
            let n = stream.read(&mut byte).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "closed during handshake",
                ));
            }
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        if !head.starts_with("HTTP/1.1 101") {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("upgrade refused: {}", head.lines().next().unwrap_or("?")),
            ));
        }
        Ok(Self { stream, rx: Vec::new() })
    }

    async fn need(&mut self, n: usize) -> io::Result<Vec<u8>> {
        while self.rx.len() < n {
            let mut chunk = [0u8; 8192];
            let r = self.stream.read(&mut chunk).await?;
            if r == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof"));
            }
            self.rx.extend_from_slice(&chunk[..r]);
        }
        let out = self.rx[..n].to_vec();
        self.rx.drain(..n);
        Ok(out)
    }

    /// Send one JSON document as a masked binary frame - the EMSC socket
    /// parses binary frames only (its web client sends UTF-8 JSON as binary
    /// too), text frames are silently ignored.
    pub async fn send_json(&mut self, v: &Value) -> io::Result<()> {
        self.send_str(&serde_json::to_string(v).unwrap_or_default()).await
    }

    /// Send a pre-serialised JSON string (key order matters to the EMSC
    /// dispatcher: `label` must come first, which serde's alphabetically
    /// ordered Value map would not guarantee).
    pub async fn send_str(&mut self, s: &str) -> io::Result<()> {
        self.send_frame(0x2, s.as_bytes().to_vec()).await
    }

    async fn send_frame(&mut self, op: u8, payload: Vec<u8>) -> io::Result<()> {
        let len = payload.len();
        let mut head = vec![0x80 | op];
        if len < 126 {
            head.push(0x80 | len as u8);
        } else if len < 65536 {
            head.push(0x80 | 126);
            head.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            head.push(0x80 | 127);
            head.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask = nonce_bytes();
        head.extend_from_slice(&mask[..4]);
        let mut masked = payload;
        for (i, b) in masked.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        head.append(&mut masked);
        self.stream.write_all(&head).await?;
        self.stream.flush().await
    }

    /// Next application message; pings are answered, close reports `Closed`.
    pub async fn recv_message(&mut self) -> io::Result<WsMessage> {
        let mut payload: Vec<u8> = Vec::new();
        let mut frag_op: Option<u8> = None;
        loop {
            let h = self.need(2).await?;
            let (b1, b2) = (h[0], h[1]);
            let fin = b1 & 0x80 != 0;
            let op = b1 & 0x0f;
            let masked = b2 & 0x80 != 0;
            let mut len = (b2 & 0x7f) as u64;
            if len == 126 {
                let e = self.need(2).await?;
                len = u16::from_be_bytes([e[0], e[1]]) as u64;
            } else if len == 127 {
                let e = self.need(8).await?;
                len = u64::from_be_bytes([e[0], e[1], e[2], e[3], e[4], e[5], e[6], e[7]]);
            }
            if len > 32 * 1024 * 1024 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
            }
            let mask = if masked { Some(self.need(4).await?) } else { None };
            let mut p = self.need(len as usize).await?;
            if let Some(m) = mask {
                for (i, b) in p.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }
            match op {
                0x8 => return Ok(WsMessage::Closed),
                0x9 => {
                    self.send_frame(0xA, p).await?;
                    continue;
                }
                0xA => continue,
                0x0 => payload.append(&mut p), // continuation
                0x1 | 0x2 => {
                    if frag_op.is_none() {
                        frag_op = Some(op);
                    }
                    payload.append(&mut p);
                }
                _ => continue,
            }
            if fin {
                let op = frag_op.take().unwrap_or(0x1);
                return Ok(if op == 0x2 {
                    WsMessage::Binary(payload)
                } else {
                    WsMessage::Text(String::from_utf8_lossy(&payload).into_owned())
                });
            }
        }
    }

    /// Next message parsed as JSON (the EMSC feed is JSON in text or binary
    /// frames; the gateway gzip-compresses binary payloads unless the browser
    /// can't decompress, so inflate when the magic bytes are present).
    pub async fn recv_json(&mut self) -> io::Result<Option<Value>> {
        match self.recv_message().await? {
            WsMessage::Closed => Ok(None),
            WsMessage::Text(t) => Ok(serde_json::from_str(&t).ok()),
            WsMessage::Binary(b) => {
                let json = if b.starts_with(&[0x1f, 0x8b]) {
                    use std::io::Read;
                    let mut plain = Vec::with_capacity(b.len() * 4);
                    flate2::read::GzDecoder::new(&b[..])
                        .read_to_end(&mut plain)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    plain
                } else {
                    b
                };
                Ok(serde_json::from_slice(&json).ok())
            }
        }
    }
}
