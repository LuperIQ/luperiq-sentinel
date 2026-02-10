#[cfg(feature = "tls")]
use std::cell::RefCell;
use std::io::{Read, Write};
#[cfg(feature = "tls")]
use std::net::TcpStream;
#[cfg(feature = "tls")]
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "tls")]
use rustls::pki_types::ServerName;
#[cfg(feature = "tls")]
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

// ── Types ───────────────────────────────────────────────────────────────────

#[cfg(feature = "tls")]
type TlsStream = StreamOwned<ClientConnection, TcpStream>;

#[cfg(feature = "tls")]
pub struct HttpClient {
    tls_config: Arc<ClientConfig>,
    cached_conn: RefCell<Option<CachedConn>>,
}

#[cfg(feature = "tls")]
struct CachedConn {
    host_port: String,
    stream: TlsStream,
}

pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum HttpError {
    InvalidUrl(String),
    Dns(String),
    Connect(String),
    Tls(String),
    Io(std::io::Error),
    Timeout,
    Protocol(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::InvalidUrl(s) => write!(f, "invalid URL: {}", s),
            HttpError::Dns(s) => write!(f, "DNS resolution failed: {}", s),
            HttpError::Connect(s) => write!(f, "connection failed: {}", s),
            HttpError::Tls(s) => write!(f, "TLS error: {}", s),
            HttpError::Io(e) => write!(f, "I/O error: {}", e),
            HttpError::Timeout => write!(f, "request timed out"),
            HttpError::Protocol(s) => write!(f, "protocol error: {}", s),
        }
    }
}

impl From<std::io::Error> for HttpError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock {
            HttpError::Timeout
        } else {
            HttpError::Io(e)
        }
    }
}

// ── URL parsing ─────────────────────────────────────────────────────────────

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Result<ParsedUrl, HttpError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| HttpError::InvalidUrl("URL must start with https://".into()))?;

    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    let (host, port) = match host_port.find(':') {
        Some(i) => {
            let h = &host_port[..i];
            let p = host_port[i + 1..]
                .parse::<u16>()
                .map_err(|_| HttpError::InvalidUrl("invalid port".into()))?;
            (h, p)
        }
        None => (host_port, 443),
    };

    if host.is_empty() {
        return Err(HttpError::InvalidUrl("empty host".into()));
    }

    Ok(ParsedUrl {
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

// ── HttpClient (TLS-based, Linux) ──────────────────────────────────────────

#[cfg(feature = "tls")]
impl HttpClient {
    pub fn new() -> Result<Self, HttpError> {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(HttpClient {
            tls_config: Arc::new(config),
            cached_conn: RefCell::new(None),
        })
    }

    pub fn post_json(
        &self,
        url: &str,
        body: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let parsed = parse_url(url)?;
        let mut headers = Vec::new();
        headers.push(("Content-Type", "application/json"));
        for (k, v) in extra_headers {
            headers.push((k, v));
        }
        self.request("POST", &parsed, Some(body.as_bytes()), &headers)
    }

    pub fn patch_json(
        &self,
        url: &str,
        body: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let parsed = parse_url(url)?;
        let mut headers = Vec::new();
        headers.push(("Content-Type", "application/json"));
        for (k, v) in extra_headers {
            headers.push((k, v));
        }
        self.request("PATCH", &parsed, Some(body.as_bytes()), &headers)
    }

    pub fn get(
        &self,
        url: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let parsed = parse_url(url)?;
        self.request("GET", &parsed, None, extra_headers)
    }

    fn connect(&self, url: &ParsedUrl) -> Result<TlsStream, HttpError> {
        let addr = format!("{}:{}", url.host, url.port);
        let tcp = TcpStream::connect(&addr).map_err(|e| HttpError::Connect(e.to_string()))?;
        tcp.set_read_timeout(Some(Duration::from_secs(30)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))?;

        let server_name = ServerName::try_from(url.host.clone())
            .map_err(|e| HttpError::Tls(format!("invalid server name: {}", e)))?;
        let conn = ClientConnection::new(self.tls_config.clone(), server_name)
            .map_err(|e| HttpError::Tls(e.to_string()))?;
        Ok(StreamOwned::new(conn, tcp))
    }

    fn request(
        &self,
        method: &str,
        url: &ParsedUrl,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let key = format!("{}:{}", url.host, url.port);

        // Try cached connection first
        let cached = self.cached_conn.borrow_mut().take();
        if let Some(conn) = cached {
            if conn.host_port == key {
                match self.send_and_read(conn.stream, method, url, body, headers) {
                    Ok((resp, stream)) => {
                        self.maybe_cache(key, &resp.headers, stream);
                        return Ok(resp);
                    }
                    Err(_) => {
                        // Stale connection — fall through to create new one
                    }
                }
            }
            // Different host or stale — drop the old connection
        }

        // New connection
        let stream = self.connect(url)?;
        let (resp, stream) = self.send_and_read(stream, method, url, body, headers)?;
        self.maybe_cache(key, &resp.headers, stream);
        Ok(resp)
    }

    fn maybe_cache(&self, key: String, headers: &[(String, String)], stream: TlsStream) {
        let close = get_header(headers, "connection")
            .map(|v| v.eq_ignore_ascii_case("close"))
            .unwrap_or(false);
        if !close {
            *self.cached_conn.borrow_mut() = Some(CachedConn {
                host_port: key,
                stream,
            });
        }
    }

    fn send_and_read(
        &self,
        mut stream: TlsStream,
        method: &str,
        url: &ParsedUrl,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<(HttpResponse, TlsStream), HttpError> {
        // Build request
        let mut req = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\n",
            method, url.path, url.host
        );
        for (k, v) in headers {
            req.push_str(k);
            req.push_str(": ");
            req.push_str(v);
            req.push_str("\r\n");
        }
        if let Some(b) = body {
            req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        req.push_str("\r\n");

        // Send
        stream.write_all(req.as_bytes())?;
        if let Some(b) = body {
            stream.write_all(b)?;
        }
        stream.flush()?;

        // Read response (content-length aware, not read-to-EOF)
        let resp = read_response_from_stream(&mut stream)?;
        Ok((resp, stream))
    }
}

// ── Stream-based response reading (keep-alive safe) ─────────────────────────

#[cfg(feature = "tls")]
fn read_response_from_stream(stream: &mut TlsStream) -> Result<HttpResponse, HttpError> {
    // Read headers byte-by-byte until \r\n\r\n
    let mut header_buf = Vec::with_capacity(4096);
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).map_err(|e| {
            if header_buf.is_empty() {
                // Connection was closed before any data — stale
                HttpError::Protocol("connection closed".into())
            } else {
                HttpError::from(e)
            }
        })?;
        header_buf.push(byte[0]);
        let len = header_buf.len();
        if len >= 4
            && header_buf[len - 4] == b'\r'
            && header_buf[len - 3] == b'\n'
            && header_buf[len - 2] == b'\r'
            && header_buf[len - 1] == b'\n'
        {
            break;
        }
        if len > 65536 {
            return Err(HttpError::Protocol("headers too large".into()));
        }
    }

    let header_end = header_buf.len() - 4;
    let header_str = std::str::from_utf8(&header_buf[..header_end])
        .map_err(|_| HttpError::Protocol("headers not valid UTF-8".into()))?;

    let mut lines = header_str.split("\r\n");

    // Status line
    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::Protocol("empty response".into()))?;
    let status = parse_status_line(status_line)?;

    // Headers
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let val = line[colon + 1..].trim().to_string();
            headers.push((key, val));
        }
    }

    // Read body based on Transfer-Encoding or Content-Length
    let body = if let Some(te) = get_header(&headers, "transfer-encoding") {
        if te.to_lowercase().contains("chunked") {
            read_chunked_from_stream(stream)?
        } else {
            Vec::new()
        }
    } else if let Some(cl) = get_header(&headers, "content-length") {
        let len: usize = cl
            .parse()
            .map_err(|_| HttpError::Protocol("invalid content-length".into()))?;
        if len > 0 {
            let mut body = vec![0u8; len];
            stream.read_exact(&mut body)?;
            body
        } else {
            Vec::new()
        }
    } else {
        // No content indicator — assume empty body for keep-alive
        Vec::new()
    };

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

#[cfg(feature = "tls")]
fn read_chunked_from_stream(stream: &mut TlsStream) -> Result<Vec<u8>, HttpError> {
    let mut result = Vec::new();
    loop {
        // Read chunk-size line
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte)?;
            line.push(byte[0]);
            let len = line.len();
            if len >= 2 && line[len - 2] == b'\r' && line[len - 1] == b'\n' {
                break;
            }
        }

        let size_str = std::str::from_utf8(&line[..line.len() - 2])
            .map_err(|_| HttpError::Protocol("chunk size not UTF-8".into()))?;
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpError::Protocol(format!("invalid chunk size: '{}'", size_hex)))?;

        if chunk_size == 0 {
            // Read trailing \r\n after final chunk
            let mut trail = [0u8; 2];
            stream.read_exact(&mut trail)?;
            break;
        }

        // Read chunk data + trailing \r\n
        let mut chunk = vec![0u8; chunk_size + 2];
        stream.read_exact(&mut chunk)?;
        result.extend_from_slice(&chunk[..chunk_size]);
    }
    Ok(result)
}

// ── Streaming response ──────────────────────────────────────────────────────

#[cfg(feature = "tls")]
pub struct StreamingResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    stream: TlsStream,
}

#[cfg(feature = "tls")]
impl StreamingResponse {
    /// Read a single line (up to \n). Returns empty string on EOF.
    pub fn read_line(&mut self) -> Result<String, HttpError> {
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            match self.stream.read_exact(&mut byte) {
                Ok(()) => {
                    if byte[0] == b'\n' {
                        return Ok(String::from_utf8_lossy(&line).into_owned());
                    }
                    line.push(byte[0]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    if line.is_empty() {
                        return Ok(String::new());
                    }
                    return Ok(String::from_utf8_lossy(&line).into_owned());
                }
                Err(e) => return Err(HttpError::from(e)),
            }
            if line.len() > 1_048_576 {
                return Err(HttpError::Protocol("line too long".into()));
            }
        }
    }
}

#[cfg(feature = "tls")]
impl HttpClient {
    /// Send a POST request and return a streaming response (for SSE).
    /// The caller reads the body incrementally via StreamingResponse::read_line.
    pub fn post_json_streaming(
        &self,
        url: &str,
        body: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<StreamingResponse, HttpError> {
        let parsed = parse_url(url)?;

        // Always create a fresh connection for streaming (don't use cache)
        let mut stream = self.connect(&parsed)?;

        // Build request
        let mut req = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n",
            parsed.path, parsed.host
        );
        for (k, v) in extra_headers {
            req.push_str(k);
            req.push_str(": ");
            req.push_str(v);
            req.push_str("\r\n");
        }
        req.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));

        stream.write_all(req.as_bytes())?;
        stream.write_all(body.as_bytes())?;
        stream.flush()?;

        // Read response headers
        let mut header_buf = Vec::with_capacity(4096);
        loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte)?;
            header_buf.push(byte[0]);
            let len = header_buf.len();
            if len >= 4
                && header_buf[len - 4] == b'\r'
                && header_buf[len - 3] == b'\n'
                && header_buf[len - 2] == b'\r'
                && header_buf[len - 1] == b'\n'
            {
                break;
            }
            if len > 65536 {
                return Err(HttpError::Protocol("headers too large".into()));
            }
        }

        let header_end = header_buf.len() - 4;
        let header_str = std::str::from_utf8(&header_buf[..header_end])
            .map_err(|_| HttpError::Protocol("headers not valid UTF-8".into()))?;

        let mut lines = header_str.split("\r\n");
        let status_line = lines.next().ok_or_else(|| HttpError::Protocol("empty response".into()))?;
        let status = parse_status_line(status_line)?;

        let mut headers = Vec::new();
        for line in lines {
            if line.is_empty() { break; }
            if let Some(colon) = line.find(':') {
                let key = line[..colon].trim().to_lowercase();
                let val = line[colon + 1..].trim().to_string();
                headers.push((key, val));
            }
        }

        Ok(StreamingResponse { status, headers, stream })
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

fn parse_status_line(line: &str) -> Result<u16, HttpError> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(HttpError::Protocol("malformed status line".into()));
    }
    parts[1]
        .parse::<u16>()
        .map_err(|_| HttpError::Protocol("invalid status code".into()))
}

fn get_header<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

impl HttpResponse {
    pub fn body_string(&self) -> Result<String, HttpError> {
        String::from_utf8(self.body.clone())
            .map_err(|_| HttpError::Protocol("response body is not valid UTF-8".into()))
    }
}

// ── Raw response parsing (used by tests) ────────────────────────────────────

fn parse_response(raw: &[u8]) -> Result<HttpResponse, HttpError> {
    let header_end = find_header_end(raw)
        .ok_or_else(|| HttpError::Protocol("no header/body boundary found".into()))?;

    let header_bytes = &raw[..header_end];
    let body_start = header_end + 4;

    let header_str = std::str::from_utf8(header_bytes)
        .map_err(|_| HttpError::Protocol("headers not valid UTF-8".into()))?;

    let mut lines = header_str.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::Protocol("empty response".into()))?;
    let status = parse_status_line(status_line)?;

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let val = line[colon + 1..].trim().to_string();
            headers.push((key, val));
        }
    }

    let raw_body = if body_start <= raw.len() {
        &raw[body_start..]
    } else {
        &[]
    };

    let body = decode_body(raw_body, &headers)?;
    Ok(HttpResponse { status, headers, body })
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(3) {
        if data[i] == b'\r' && data[i + 1] == b'\n' && data[i + 2] == b'\r' && data[i + 3] == b'\n'
        {
            return Some(i);
        }
    }
    None
}

fn decode_body(raw: &[u8], headers: &[(String, String)]) -> Result<Vec<u8>, HttpError> {
    if let Some(te) = get_header(headers, "transfer-encoding") {
        if te.to_lowercase().contains("chunked") {
            return decode_chunked(raw);
        }
    }
    if let Some(cl) = get_header(headers, "content-length") {
        let len: usize = cl
            .parse()
            .map_err(|_| HttpError::Protocol("invalid content-length".into()))?;
        if raw.len() >= len {
            return Ok(raw[..len].to_vec());
        }
    }
    Ok(raw.to_vec())
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut result = Vec::new();
    let mut pos = 0;
    loop {
        let line_end = find_crlf(data, pos)
            .ok_or_else(|| HttpError::Protocol("malformed chunked data".into()))?;
        let size_str = std::str::from_utf8(&data[pos..line_end])
            .map_err(|_| HttpError::Protocol("chunk size not UTF-8".into()))?
            .trim();
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpError::Protocol(format!("invalid chunk size: '{}'", size_hex)))?;
        pos = line_end + 2;
        if chunk_size == 0 {
            break;
        }
        if pos + chunk_size > data.len() {
            result.extend_from_slice(&data[pos..]);
            break;
        }
        result.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size + 2;
    }
    Ok(result)
}

fn find_crlf(data: &[u8], start: usize) -> Option<usize> {
    for i in start..data.len().saturating_sub(1) {
        if data[i] == b'\r' && data[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_basic() {
        let url = parse_url("https://api.example.com/v1/messages").unwrap();
        assert_eq!(url.host, "api.example.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.path, "/v1/messages");
    }

    #[test]
    fn test_parse_url_with_port() {
        let url = parse_url("https://localhost:8443/test").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, 8443);
        assert_eq!(url.path, "/test");
    }

    #[test]
    fn test_parse_url_no_path() {
        let url = parse_url("https://example.com").unwrap();
        assert_eq!(url.host, "example.com");
        assert_eq!(url.path, "/");
    }

    #[test]
    fn test_parse_url_rejects_http() {
        assert!(parse_url("http://example.com").is_err());
    }

    #[test]
    fn test_parse_response_basic() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"hello");
    }

    #[test]
    fn test_parse_response_404() {
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body_string().unwrap(), "not found");
    }

    #[test]
    fn test_parse_response_chunked() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.body_string().unwrap(), "hello world");
    }

    #[test]
    fn test_parse_response_headers() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nX-Custom: test\r\n\r\n{}";
        let resp = parse_response(raw).unwrap();
        assert_eq!(get_header(&resp.headers, "content-type"), Some("application/json"));
        assert_eq!(get_header(&resp.headers, "x-custom"), Some("test"));
    }

    #[test]
    fn test_find_header_end() {
        let data = b"Header1: val\r\nHeader2: val\r\n\r\nbody";
        assert_eq!(find_header_end(data), Some(26));
    }

    #[test]
    fn test_decode_chunked() {
        let data = b"3\r\nabc\r\n4\r\ndefg\r\n0\r\n\r\n";
        let result = decode_chunked(data).unwrap();
        assert_eq!(result, b"abcdefg");
    }
}
