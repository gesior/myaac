use std::env;
use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PHASE_WARMUP: u8 = 0;
const PHASE_MEASURE: u8 = 1;
const PHASE_STOP: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Hit,
    Bad,
    Miss,
}

struct Args {
    url: String,
    mode: Mode,
    accounts: u64,
    warmup: Duration,
    duration: Duration,
    threads: usize,
    connections: usize,
    health: Option<String>,
}

struct Stats {
    matched: u64,
    mismatch: u64,
    io_errors: u64,
    latencies_us: Vec<u64>,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "usage: login-bench --url URL --mode hit|bad|miss [--accounts 10000] [--warmup 10] [--duration 30] [--threads 12] [--connections 256] [--health URL]"
            );
            return ExitCode::from(2);
        }
    };

    let target = match Target::parse(&args.url) {
        Ok(target) => target,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    println!("cpus_allowed: {}", cpus_allowed());
    println!("url: {}", args.url);
    println!("mode: {}", args.mode.name());
    println!("threads: {}", args.threads);
    println!("connections: {}", args.connections);
    println!("accounts: {}", args.accounts);
    let _ = io::stdout().flush();

    let shared = Arc::new(Shared {
        phase: AtomicU8::new(PHASE_WARMUP),
        seq: AtomicU64::new(0),
    });
    let mut handles = Vec::with_capacity(args.threads);
    for id in 0..args.threads {
        let extra = usize::from(id < args.connections % args.threads);
        let connections = args.connections / args.threads + extra;
        let shared = Arc::clone(&shared);
        let target = target.clone();
        let mode = args.mode;
        let accounts = args.accounts;
        handles.push(thread::spawn(move || {
            worker(id, connections, shared, target, mode, accounts)
        }));
    }

    println!("warmup_s: {}", args.warmup.as_secs());
    let _ = io::stdout().flush();
    thread::sleep(args.warmup);
    let queries_before = args.health.as_deref().and_then(character_queries);
    shared.phase.store(PHASE_MEASURE, Ordering::Release);

    println!("measure_s: {}", args.duration.as_secs());
    let _ = io::stdout().flush();
    let started = Instant::now();
    thread::sleep(args.duration);
    shared.phase.store(PHASE_STOP, Ordering::Release);

    let mut matched = 0u64;
    let mut mismatch = 0u64;
    let mut io_errors = 0u64;
    let mut latencies = Vec::new();
    for handle in handles {
        let stats = handle.join().expect("worker panicked");
        matched += stats.matched;
        mismatch += stats.mismatch;
        io_errors += stats.io_errors;
        latencies.extend(stats.latencies_us);
    }
    let elapsed = started.elapsed().as_secs_f64();
    let queries_after = args.health.as_deref().and_then(character_queries);

    latencies.sort_unstable();
    let completed = matched + mismatch;
    let rps = if elapsed > 0.0 {
        completed as f64 / elapsed
    } else {
        0.0
    };
    let ratio = if completed == 0 {
        0.0
    } else {
        matched as f64 / completed as f64
    };

    println!("requests: {completed}");
    println!("matched: {matched}");
    println!("mismatch: {mismatch}");
    println!("io_errors: {io_errors}");
    println!("rps: {rps:.1}");
    println!("p50_ms: {:.3}", percentile(&latencies, 0.50));
    println!("p99_ms: {:.3}", percentile(&latencies, 0.99));
    println!("match_ratio: {ratio:.4}");
    if let (Some(before), Some(after)) = (queries_before, queries_after) {
        println!(
            "character_queries_measured: {}",
            after.saturating_sub(before)
        );
    }

    if completed == 0 || ratio < 0.99 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

struct Shared {
    phase: AtomicU8,
    seq: AtomicU64,
}

fn worker(
    _id: usize,
    connections: usize,
    shared: Arc<Shared>,
    target: Target,
    mode: Mode,
    accounts: u64,
) -> Stats {
    let mut slots = Vec::with_capacity(connections);
    for _ in 0..connections {
        slots.push(connect_slot(&target));
    }
    let mut next_slot = 0usize;
    let mut stats = Stats {
        matched: 0,
        mismatch: 0,
        io_errors: 0,
        latencies_us: Vec::new(),
    };
    if slots.is_empty() {
        return stats;
    }

    loop {
        let phase = shared.phase.load(Ordering::Acquire);
        if phase == PHASE_STOP {
            break;
        }
        let n = shared.seq.fetch_add(1, Ordering::Relaxed);
        let request = build_request(&target, mode, accounts, n);
        let index = next_slot;
        next_slot = (next_slot + 1) % slots.len();
        if slots[index].is_none() {
            slots[index] = connect_slot(&target);
        }
        if slots[index].is_none() {
            stats.io_errors += 1;
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        let started = Instant::now();
        let outcome = exchange(slots[index].as_mut().unwrap(), &request);
        let drop_slot = match outcome {
            Ok(body) => {
                if phase == PHASE_MEASURE {
                    let micros = started.elapsed().as_micros() as u64;
                    if is_match(mode, &body) {
                        stats.matched += 1;
                    } else {
                        stats.mismatch += 1;
                    }
                    stats.latencies_us.push(micros);
                }
                slots[index].as_ref().unwrap().close_after
            }
            Err(_) => {
                if phase == PHASE_MEASURE {
                    stats.io_errors += 1;
                }
                true
            }
        };
        if drop_slot {
            slots[index] = None;
        }
    }
    stats
}

struct Slot {
    stream: TcpStream,
    buf: Vec<u8>,
    close_after: bool,
}

fn connect_slot(target: &Target) -> Option<Slot> {
    let stream = TcpStream::connect_timeout(&target.addr, Duration::from_secs(2)).ok()?;
    stream.set_nodelay(true).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .ok()?;
    Some(Slot {
        stream,
        buf: Vec::with_capacity(4096),
        close_after: false,
    })
}

fn exchange(slot: &mut Slot, request: &[u8]) -> io::Result<Vec<u8>> {
    slot.close_after = false;
    slot.stream.write_all(request)?;
    loop {
        match take_response(&mut slot.buf, false)? {
            Some((body, close)) => {
                slot.close_after = close;
                return Ok(body);
            }
            None => {
                let mut tmp = [0u8; 8192];
                let n = slot.stream.read(&mut tmp)?;
                if n == 0 {
                    return match take_response(&mut slot.buf, true)? {
                        Some((body, close)) => {
                            slot.close_after = close;
                            Ok(body)
                        }
                        None => Err(io::Error::new(
                            ErrorKind::UnexpectedEof,
                            "connection closed",
                        )),
                    };
                }
                slot.buf.extend_from_slice(&tmp[..n]);
            }
        }
    }
}

fn take_response(buf: &mut Vec<u8>, eof: bool) -> io::Result<Option<(Vec<u8>, bool)>> {
    let Some(header_end) = find_slice(buf, b"\r\n\r\n") else {
        if eof && !buf.is_empty() {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "connection closed before headers",
            ));
        }
        return Ok(None);
    };
    let close = response_closes(&buf[..header_end]);
    let body_at = header_end + 4;
    if transfer_chunked(&buf[..header_end]) {
        return match decode_chunked(&buf[body_at..])? {
            Some((body, used)) => {
                buf.drain(..body_at + used);
                Ok(Some((body, close)))
            }
            None if eof => Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "connection closed during chunked body",
            )),
            None => Ok(None),
        };
    }
    if let Some(length) = content_length(&buf[..header_end]) {
        if length > 1_048_576 {
            return Err(io::Error::new(ErrorKind::InvalidData, "response too large"));
        }
        if buf.len() < body_at + length {
            if eof {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "connection closed during body",
                ));
            }
            return Ok(None);
        }
        let body = buf[body_at..body_at + length].to_vec();
        buf.drain(..body_at + length);
        return Ok(Some((body, close)));
    }
    if !close {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "response has no content-length",
        ));
    }
    if !eof {
        return Ok(None);
    }
    let body = buf[body_at..].to_vec();
    buf.clear();
    Ok(Some((body, true)))
}

fn decode_chunked(input: &[u8]) -> io::Result<Option<(Vec<u8>, usize)>> {
    let mut pos = 0;
    let mut body = Vec::new();
    loop {
        let Some(line_end) = find_slice(&input[pos..], b"\r\n") else {
            return Ok(None);
        };
        let line = std::str::from_utf8(&input[pos..pos + line_end]).map_err(|_| {
            io::Error::new(ErrorKind::InvalidData, "chunk size is not utf-8")
        })?;
        let size_hex = line.split(';').next().unwrap_or(line).trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| {
            io::Error::new(ErrorKind::InvalidData, "bad chunk size")
        })?;
        if body.len().saturating_add(size) > 1_048_576 {
            return Err(io::Error::new(ErrorKind::InvalidData, "response too large"));
        }
        let data_at = pos + line_end + 2;
        if size == 0 {
            let rest = &input[data_at..];
            if rest.starts_with(b"\r\n") {
                return Ok(Some((body, data_at + 2)));
            }
            if let Some(blank) = find_slice(rest, b"\r\n\r\n") {
                return Ok(Some((body, data_at + blank + 4)));
            }
            return Ok(None);
        }
        if input.len() < data_at + size + 2 {
            return Ok(None);
        }
        if &input[data_at + size..data_at + size + 2] != b"\r\n" {
            return Err(io::Error::new(ErrorKind::InvalidData, "chunk is not terminated"));
        }
        body.extend_from_slice(&input[data_at..data_at + size]);
        pos = data_at + size + 2;
    }
}

fn transfer_chunked(headers: &[u8]) -> bool {
    header_value(headers, "transfer-encoding")
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(|value| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

fn response_closes(headers: &[u8]) -> bool {
    let connection = header_value(headers, "connection")
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(|value| value.trim().to_ascii_lowercase());
    match connection.as_deref() {
        Some("close") => true,
        Some("keep-alive") => false,
        _ => headers.starts_with(b"HTTP/1.0"),
    }
}

fn content_length(headers: &[u8]) -> Option<usize> {
    let value = header_value(headers, "content-length")?;
    std::str::from_utf8(value).ok()?.trim().parse().ok()
}

fn header_value<'a>(headers: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let mut start = 0;
    while start < headers.len() {
        let end = headers[start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|pos| start + pos)
            .unwrap_or(headers.len());
        let mut line = &headers[start..end];
        if let Some(stripped) = line.strip_suffix(b"\r") {
            line = stripped;
        }
        if let Some(colon) = line.iter().position(|&byte| byte == b':') {
            if eq_ignore_ascii_case(trim_ascii(&line[..colon]), name.as_bytes()) {
                return Some(trim_ascii(&line[colon + 1..]));
            }
        }
        if end >= headers.len() {
            break;
        }
        start = end + 1;
    }
    None
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|pos| pos + 1)
        .unwrap_or(start);
    &bytes[start..end]
}

fn eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}

fn find_slice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[derive(Clone)]
struct Target {
    addr: SocketAddr,
    host_header: String,
    path: String,
}

impl Target {
    fn parse(url: &str) -> Result<Self, String> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| format!("only http:// URLs are supported, got {url}"))?;
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], rest[index..].to_string()),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() || path.is_empty() {
            return Err(format!("invalid URL {url}"));
        }
        let (host, port) = if let Some(host) = authority.strip_prefix('[') {
            let end = host
                .find(']')
                .ok_or_else(|| format!("invalid IPv6 URL {url}"))?;
            let host = &host[..end];
            let after = &authority[end + 2..];
            let port = if after.is_empty() {
                80
            } else {
                after
                    .strip_prefix(':')
                    .ok_or_else(|| format!("invalid URL {url}"))?
                    .parse::<u16>()
                    .map_err(|_| format!("invalid port in {url}"))?
            };
            (host.to_string(), port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("invalid port in {url}"))?;
            (host.to_string(), port)
        } else {
            (authority.to_string(), 80)
        };
        let addr = format!("{host}:{port}")
            .parse::<SocketAddr>()
            .map_err(|_| format!("URL host must be a numeric address, got {host}"))?;
        Ok(Self {
            addr,
            host_header: format!("{host}:{port}"),
            path,
        })
    }
}

fn build_request(target: &Target, mode: Mode, accounts: u64, seq: u64) -> Vec<u8> {
    let id = (seq % accounts) + 1;
    let name = match mode {
        Mode::Hit | Mode::Bad => format!("bench{id:05}"),
        Mode::Miss => format!("miss{id:05}"),
    };
    let password = match mode {
        Mode::Hit | Mode::Miss => "secret",
        Mode::Bad => "wrong",
    };
    let body = format!(
        "{{\"type\":\"login\",\"accountname\":\"{name}\",\"password\":\"{password}\"}}"
    );
    let header = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n",
        target.path,
        target.host_header,
        body.len()
    );
    let mut request = Vec::with_capacity(header.len() + body.len());
    request.extend_from_slice(header.as_bytes());
    request.extend_from_slice(body.as_bytes());
    request
}

fn is_match(mode: Mode, body: &[u8]) -> bool {
    let session = find_slice(body, b"sessionkey").is_some();
    let error = find_slice(body, b"errorMessage").is_some();
    match mode {
        Mode::Hit => session && !error,
        Mode::Bad | Mode::Miss => error && !session,
    }
}

fn character_queries(url: &str) -> Option<u64> {
    let target = Target::parse(url).ok()?;
    let mut slot = connect_slot(&target)?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        target.path, target.host_header
    );
    let body = exchange(&mut slot, request.as_bytes()).ok()?;
    let text = std::str::from_utf8(&body).ok()?;
    let marker = "\"character_queries\":";
    let start = text.find(marker)? + marker.len();
    let digits: String = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn percentile(sorted_us: &[u64], p: f64) -> f64 {
    if sorted_us.is_empty() {
        return 0.0;
    }
    let index = ((sorted_us.len() - 1) as f64 * p).round() as usize;
    sorted_us[index] as f64 / 1000.0
}

fn cpus_allowed() -> String {
    let Ok(text) = fs::read_to_string("/proc/self/status") else {
        return "unknown".to_string();
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Cpus_allowed_list:") {
            return rest.trim().to_string();
        }
    }
    "unknown".to_string()
}

fn parse_args() -> Result<Args, String> {
    let mut url = None;
    let mut mode = None;
    let mut accounts = 10_000u64;
    let mut warmup = 10u64;
    let mut duration = 30u64;
    let mut threads = 12usize;
    let mut connections = 256usize;
    let mut health = None;
    let mut args = env::args().skip(1);
    while let Some(key) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        match key.as_str() {
            "--url" => url = Some(value),
            "--mode" => {
                mode = Some(match value.as_str() {
                    "hit" => Mode::Hit,
                    "bad" => Mode::Bad,
                    "miss" => Mode::Miss,
                    other => return Err(format!("unknown mode {other}")),
                });
            }
            "--accounts" => accounts = parse_num(&key, &value)?,
            "--warmup" => warmup = parse_num(&key, &value)?,
            "--duration" => duration = parse_num(&key, &value)?,
            "--threads" => threads = parse_num(&key, &value)?,
            "--connections" => connections = parse_num(&key, &value)?,
            "--health" => health = Some(value),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if accounts == 0 || threads == 0 || connections == 0 || duration == 0 {
        return Err("accounts, threads, connections and duration must be > 0".into());
    }
    if accounts > 99_999 {
        return Err("accounts must be <= 99999 so names stay bench00001-style".into());
    }
    if connections < threads {
        connections = threads;
    }
    Ok(Args {
        url: url.ok_or("missing --url")?,
        mode: mode.ok_or("missing --mode")?,
        accounts,
        warmup: Duration::from_secs(warmup),
        duration: Duration::from_secs(duration),
        threads,
        connections,
        health,
    })
}

fn parse_num<T: std::str::FromStr>(key: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("invalid number for {key}: {value}"))
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Bad => "bad",
            Self::Miss => "miss",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_match_rules() {
        let target = Target::parse("http://127.0.0.1:8080/login.php").unwrap();
        let hit = build_request(&target, Mode::Hit, 10_000, 0);
        let text = String::from_utf8(hit).unwrap();
        assert!(text.contains("\"accountname\":\"bench00001\""));
        assert!(text.contains("\"password\":\"secret\""));
        assert!(text.contains("POST /login.php HTTP/1.1\r\n"));

        let miss = String::from_utf8(build_request(&target, Mode::Miss, 10_000, 41)).unwrap();
        assert!(miss.contains("\"accountname\":\"miss00042\""));

        let bad = String::from_utf8(build_request(&target, Mode::Bad, 10_000, 0)).unwrap();
        assert!(bad.contains("\"password\":\"wrong\""));

        assert!(is_match(Mode::Hit, br#"{"session":{"sessionkey":"a"}}"#));
        assert!(!is_match(
            Mode::Hit,
            br#"{"errorCode":3,"errorMessage":"no"}"#
        ));
        assert!(is_match(
            Mode::Bad,
            br#"{"errorCode":3,"errorMessage":"Account name or password is not correct."}"#
        ));
        assert!(is_match(
            Mode::Miss,
            br#"{"errorMessage":"Account name or password is not correct."}"#
        ));
        assert!(!is_match(Mode::Bad, br#"{"sessionkey":"x"}"#));
    }

    #[test]
    fn parses_content_length_and_percentiles() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhelloEXTRA";
        let mut buf = raw.to_vec();
        let (body, close) = take_response(&mut buf, false).unwrap().unwrap();
        assert_eq!(body, b"hello");
        assert!(close);
        assert_eq!(buf, b"EXTRA");

        let mut chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec();
        let (body, close) = take_response(&mut chunked, false).unwrap().unwrap();
        assert_eq!(body, b"hello");
        assert!(!close);
        assert!(chunked.is_empty());

        let mut until_close = b"HTTP/1.0 200 OK\r\n\r\nhello".to_vec();
        assert!(take_response(&mut until_close, false).unwrap().is_none());
        let (body, close) = take_response(&mut until_close, true).unwrap().unwrap();
        assert_eq!(body, b"hello");
        assert!(close);
        assert_eq!(percentile(&[100, 200, 300, 400], 0.5), 0.3);
    }
}
