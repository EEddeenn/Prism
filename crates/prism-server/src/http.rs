//! A minimal, defensive HTTP/1.1 surface in pure std.
//!
//! Just enough to serve a read-only local UI: parse the request line and
//! headers of a GET, enforce a header-size cap, and write JSON or static
//! responses. Bodies are ignored (GET only). Nothing here panics on malformed
//! input — a bad request is reported as [`ParseError`] and the caller answers
//! `400`.

use std::io::{self, BufRead, BufReader, Read, Write};

/// The largest request head (request line + headers) we will buffer, in bytes.
/// A read that exceeds this is rejected before any allocation blows up.
pub const MAX_HEAD_BYTES: usize = 16 * 1024;

/// The largest single header line we accept.
const MAX_LINE_BYTES: usize = 8 * 1024;

/// A parsed request line plus the path/query split out. Headers are read and
/// validated but, for a read-only GET server, otherwise unused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The HTTP method, uppercased as received (we only serve `GET`).
    pub method: String,
    /// The raw request target (everything before the first space-delimited
    /// version token), e.g. `/api/graph/stats?run=1`.
    pub target: String,
    /// The path portion of `target` (before `?`), percent-decoded.
    pub path: String,
    /// The raw query string (after `?`), or empty. Not yet split into params.
    pub raw_query: String,
}

/// Why a request could not be parsed. Every variant maps to a `400`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The connection closed before a full request line arrived.
    Empty,
    /// The request line was not `METHOD TARGET VERSION`.
    MalformedRequestLine,
    /// The request head exceeded [`MAX_HEAD_BYTES`] (or a line was too long).
    TooLarge,
    /// A header line was not `Name: value`.
    MalformedHeader,
    /// The percent-encoding in the target was invalid.
    BadPercentEncoding,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ParseError::Empty => "empty request",
            ParseError::MalformedRequestLine => "malformed request line",
            ParseError::TooLarge => "request head too large",
            ParseError::MalformedHeader => "malformed header",
            ParseError::BadPercentEncoding => "invalid percent-encoding",
        };
        f.write_str(s)
    }
}

impl Request {
    /// Parse a request from a reader: the request line, then headers up to the
    /// blank line. The body (if any) is left unread on the stream.
    ///
    /// Caps total head bytes at [`MAX_HEAD_BYTES`] and each line at
    /// `MAX_LINE_BYTES`; an I/O error surfaces as `Err(Ok(io::Error))`-style via
    /// the `io::Result` wrapping. A structurally bad request is `Ok(Err(_))`.
    pub fn read_from<R: Read>(reader: R) -> io::Result<Result<Request, ParseError>> {
        let mut buf = BufReader::new(reader);

        // Request line.
        let mut line = Vec::new();
        if read_line_capped(&mut buf, &mut line, MAX_LINE_BYTES)? == 0 {
            return Ok(Err(ParseError::Empty));
        }
        let request = match parse_request_line(&line) {
            Ok(r) => r,
            Err(e) => return Ok(Err(e)),
        };

        // Headers: read until a blank line, enforcing the total head budget.
        let mut consumed = line.len();
        loop {
            line.clear();
            let n = read_line_capped(&mut buf, &mut line, MAX_LINE_BYTES)?;
            if n == 0 {
                // EOF before the blank line: tolerate it (treat as end of head).
                break;
            }
            consumed += n;
            if consumed > MAX_HEAD_BYTES {
                return Ok(Err(ParseError::TooLarge));
            }
            let trimmed = trim_crlf(&line);
            if trimmed.is_empty() {
                break; // end of headers
            }
            // A header must contain a colon; we don't otherwise use the value.
            if !trimmed.contains(&b':') {
                return Ok(Err(ParseError::MalformedHeader));
            }
        }

        Ok(Ok(request))
    }
}

/// Parse `METHOD TARGET VERSION\r\n` into a [`Request`] (path/query split).
fn parse_request_line(line: &[u8]) -> Result<Request, ParseError> {
    let line = trim_crlf(line);
    // The request line must be valid ASCII (control chars already excluded by
    // CRLF trimming would still let through e.g. NUL; reject non-ASCII outright).
    let text = std::str::from_utf8(line).map_err(|_| ParseError::MalformedRequestLine)?;
    let mut parts = text.split(' ');
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    // Exactly three space-separated tokens, and no extras.
    if method.is_empty()
        || target.is_empty()
        || version.is_empty()
        || parts.next().is_some()
        || !version.starts_with("HTTP/")
    {
        return Err(ParseError::MalformedRequestLine);
    }

    let (raw_path, raw_query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };
    let path = percent_decode(raw_path).ok_or(ParseError::BadPercentEncoding)?;

    Ok(Request {
        method: method.to_ascii_uppercase(),
        target: target.to_string(),
        path,
        raw_query: raw_query.to_string(),
    })
}

/// Read one `\n`-terminated line into `out`, returning the byte count read (0 at
/// EOF). Rejects lines longer than `cap` as an [`io::ErrorKind::InvalidData`] so
/// the caller surfaces a `400` rather than buffering unbounded input.
fn read_line_capped<R: BufRead>(reader: &mut R, out: &mut Vec<u8>, cap: usize) -> io::Result<usize> {
    let mut total = 0usize;
    loop {
        let available = match reader.fill_buf() {
            Ok(b) => b,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            break; // EOF
        }
        let found_newline = match available.iter().position(|&b| b == b'\n') {
            Some(i) => {
                out.extend_from_slice(&available[..=i]);
                total += i + 1;
                reader.consume(i + 1);
                true
            }
            None => {
                out.extend_from_slice(available);
                let len = available.len();
                total += len;
                reader.consume(len);
                false
            }
        };
        // Enforce the per-line cap in both branches, including the terminating
        // chunk: a single over-long line must not slip through.
        if total > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        if found_newline {
            break;
        }
    }
    Ok(total)
}

/// Strip a trailing `\r\n` or `\n` from a line.
fn trim_crlf(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
        if end > 0 && line[end - 1] == b'\r' {
            end -= 1;
        }
    }
    &line[..end]
}

/// Percent-decode a path segment, returning `None` on a malformed escape. Plus
/// signs are *not* treated as spaces in the path (that is a query-string rule).
pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hi = *bytes.get(i + 1)?;
                let lo = *bytes.get(i + 2)?;
                let byte = (hex_val(hi)? << 4) | hex_val(lo)?;
                out.push(byte);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Hex digit value, or `None` for a non-hex byte.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// An HTTP status code we emit, paired with its reason phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status(pub u16, pub &'static str);

impl Status {
    /// `200 OK`.
    pub const OK: Status = Status(200, "OK");
    /// `204 No Content` (used for CORS preflight replies).
    pub const NO_CONTENT: Status = Status(204, "No Content");
    /// `400 Bad Request`.
    pub const BAD_REQUEST: Status = Status(400, "Bad Request");
    /// `404 Not Found`.
    pub const NOT_FOUND: Status = Status(404, "Not Found");
    /// `405 Method Not Allowed`.
    pub const METHOD_NOT_ALLOWED: Status = Status(405, "Method Not Allowed");
    /// `500 Internal Server Error`.
    pub const INTERNAL_ERROR: Status = Status(500, "Internal Server Error");
}

/// A fully-formed response: status, content type, and a body.
pub struct Response {
    /// The status line code/phrase.
    pub status: Status,
    /// The `Content-Type` header value.
    pub content_type: &'static str,
    /// The response body bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// A JSON response with the given status.
    pub fn json(status: Status, body: String) -> Response {
        Response {
            status,
            content_type: "application/json; charset=utf-8",
            body: body.into_bytes(),
        }
    }

    /// An empty `204 No Content` response — the reply to a CORS preflight.
    pub fn no_content() -> Response {
        Response {
            status: Status::NO_CONTENT,
            content_type: "text/plain; charset=utf-8",
            body: Vec::new(),
        }
    }

    /// Serialize the response to `w` as HTTP/1.1 with `Connection: close`.
    ///
    /// CORS is wide-open (`Access-Control-Allow-Origin: *`): this service binds
    /// loopback only and is strictly read-only, so any local origin (the Next.js
    /// dev server, another tool) may call the JSON API directly. Only `GET` is
    /// ever served, so preflight advertises just `GET, OPTIONS`.
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        let head = format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             X-Content-Type-Options: nosniff\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Access-Control-Allow-Methods: GET, OPTIONS\r\n\
             Access-Control-Allow-Headers: *\r\n\
             Access-Control-Max-Age: 86400\r\n\
             \r\n",
            self.status.0,
            self.status.1,
            self.content_type,
            self.body.len(),
        );
        w.write_all(head.as_bytes())?;
        w.write_all(&self.body)?;
        w.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(req: &str) -> Result<Request, ParseError> {
        Request::read_from(req.as_bytes()).expect("no io error")
    }

    #[test]
    fn parses_simple_get() {
        let r = parse("GET /api/status HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/api/status");
        assert_eq!(r.raw_query, "");
        assert_eq!(r.target, "/api/status");
    }

    #[test]
    fn splits_path_and_query() {
        let r = parse("GET /api/graph/stats?run=7&x=1 HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(r.path, "/api/graph/stats");
        assert_eq!(r.raw_query, "run=7&x=1");
    }

    #[test]
    fn percent_decodes_path() {
        let r = parse("GET /a%2Fb%20c HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(r.path, "/a/b c");
    }

    #[test]
    fn lowercase_method_is_uppercased() {
        let r = parse("get / HTTP/1.0\r\n\r\n").unwrap();
        assert_eq!(r.method, "GET");
    }

    #[test]
    fn empty_input_is_empty_error() {
        assert_eq!(parse(""), Err(ParseError::Empty));
    }

    #[test]
    fn missing_version_is_malformed() {
        assert_eq!(parse("GET /\r\n\r\n"), Err(ParseError::MalformedRequestLine));
    }

    #[test]
    fn extra_token_is_malformed() {
        assert_eq!(
            parse("GET / HTTP/1.1 extra\r\n\r\n"),
            Err(ParseError::MalformedRequestLine)
        );
    }

    #[test]
    fn non_http_version_is_malformed() {
        assert_eq!(parse("GET / SPDY/1\r\n\r\n"), Err(ParseError::MalformedRequestLine));
    }

    #[test]
    fn header_without_colon_is_malformed() {
        assert_eq!(
            parse("GET / HTTP/1.1\r\nbadheader\r\n\r\n"),
            Err(ParseError::MalformedHeader)
        );
    }

    #[test]
    fn bad_percent_encoding_is_rejected() {
        assert_eq!(parse("GET /%ZZ HTTP/1.1\r\n\r\n"), Err(ParseError::BadPercentEncoding));
        assert_eq!(parse("GET /% HTTP/1.1\r\n\r\n"), Err(ParseError::BadPercentEncoding));
    }

    #[test]
    fn oversized_head_is_too_large() {
        // One valid request line, then a huge run of header lines.
        let mut s = String::from("GET / HTTP/1.1\r\n");
        for _ in 0..4000 {
            s.push_str("X-Pad: 0123456789\r\n");
        }
        s.push_str("\r\n");
        assert_eq!(parse(&s), Err(ParseError::TooLarge));
    }

    #[test]
    fn oversized_single_line_is_io_rejected() {
        // A single line longer than MAX_LINE_BYTES surfaces as an io error, which
        // the caller treats as a 400.
        let mut s = String::from("GET /");
        s.push_str(&"a".repeat(MAX_LINE_BYTES + 10));
        s.push_str(" HTTP/1.1\r\n\r\n");
        let err = Request::read_from(s.as_bytes());
        assert!(err.is_err(), "expected io error for an over-long line");
    }

    #[test]
    fn eof_before_blank_line_is_tolerated() {
        // No trailing blank line; we accept the request anyway.
        let r = parse("GET / HTTP/1.1\r\nHost: x\r\n").unwrap();
        assert_eq!(r.path, "/");
    }

    #[test]
    fn response_serializes_with_headers() {
        let resp = Response::json(Status::OK, "{}".to_string());
        let mut out = Vec::new();
        resp.write_to(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.contains("Content-Type: application/json; charset=utf-8\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.contains("Access-Control-Allow-Origin: *\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn no_content_response_is_empty_204() {
        let mut out = Vec::new();
        Response::no_content().write_to(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(text.contains("Content-Length: 0\r\n"));
        assert!(text.contains("Access-Control-Allow-Methods: GET, OPTIONS\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn percent_decode_rejects_truncated_escape() {
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_decode("abc%"), None);
        assert_eq!(percent_decode("ok%41"), Some("okA".to_string()));
    }
}
