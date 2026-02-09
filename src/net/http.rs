use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

// ── Types ───────────────────────────────────────────────────────────────────

pub struct HttpClient {
    tls_config: Arc<ClientConfig>,
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

// ── HttpClient ──────────────────────────────────────────────────────────────

impl HttpClient {
    pub fn new() -> Result<Self, HttpError> {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(HttpClient {
            tls_config: Arc::new(config),
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

    pub fn get(
        &self,
        url: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        let parsed = parse_url(url)?;
        self.request("GET", &parsed, None, extra_headers)
    }

    fn request(
        &self,
        method: &str,
        url: &ParsedUrl,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, HttpError> {
        // Connect TCP
        let addr = format!("{}:{}", url.host, url.port);
        let tcp = TcpStream::connect(&addr).map_err(|e| HttpError::Connect(e.to_string()))?;
        tcp.set_read_timeout(Some(Duration::from_secs(30)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))?;

        // TLS handshake
        let server_name = ServerName::try_from(url.host.clone())
            .map_err(|e| HttpError::Tls(format!("invalid server name: {}", e)))?;
        let conn = ClientConnection::new(self.tls_config.clone(), server_name)
            .map_err(|e| HttpError::Tls(e.to_string()))?;
        let mut tls = StreamOwned::new(conn, tcp);

        // Build request
        let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n", method, url.path, url.host);

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

        // Send request
        tls.write_all(req.as_bytes())?;
        if let Some(b) = body {
            tls.write_all(b)?;
        }
        tls.flush()?;

        // Read entire response
        let mut raw = Vec::new();
        loop {
            let mut buf = [0u8; 8192];
            match tls.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
                Err(e) => return Err(e.into()),
            }
        }

        parse_response(&raw)
    }
}

// ── Response parsing ────────────────────────────────────────────────────────

fn parse_response(raw: &[u8]) -> Result<HttpResponse, HttpError> {
    // Find header/body boundary
    let header_end = find_header_end(raw)
        .ok_or_else(|| HttpError::Protocol("no header/body boundary found".into()))?;

    let header_bytes = &raw[..header_end];
    let body_start = header_end + 4; // skip \r\n\r\n

    let header_str = std::str::from_utf8(header_bytes)
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

    // Body
    let raw_body = if body_start <= raw.len() {
        &raw[body_start..]
    } else {
        &[]
    };

    let body = decode_body(raw_body, &headers)?;

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
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

fn parse_status_line(line: &str) -> Result<u16, HttpError> {
    // "HTTP/1.1 200 OK"
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

    // Fall through: take everything
    Ok(raw.to_vec())
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut result = Vec::new();
    let mut pos = 0;

    loop {
        // Find end of chunk size line
        let line_end = find_crlf(data, pos)
            .ok_or_else(|| HttpError::Protocol("malformed chunked data".into()))?;

        let size_str = std::str::from_utf8(&data[pos..line_end])
            .map_err(|_| HttpError::Protocol("chunk size not UTF-8".into()))?
            .trim();

        // Chunk extensions (after ;) are ignored
        let size_hex = size_str.split(';').next().unwrap_or("").trim();

        let chunk_size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| HttpError::Protocol(format!("invalid chunk size: '{}'", size_hex)))?;

        pos = line_end + 2; // skip \r\n

        if chunk_size == 0 {
            break;
        }

        if pos + chunk_size > data.len() {
            // Partial chunk — take what we have
            result.extend_from_slice(&data[pos..]);
            break;
        }

        result.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size + 2; // skip chunk data + \r\n
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

impl HttpResponse {
    pub fn body_string(&self) -> Result<String, HttpError> {
        String::from_utf8(self.body.clone())
            .map_err(|_| HttpError::Protocol("response body is not valid UTF-8".into()))
    }
}
