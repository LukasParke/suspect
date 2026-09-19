//! A single-request curl adapter. All redirect/auth decisions belong to this
//! module, not curl. A child can be killed even while DNS or headers are stalled.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use suspect_source::Uri;
use url::{Host, Url};

use super::{
    AcquireError, AcquireErrorKind, AcquireOptions, Budget, Context, RedirectHop, ResourcePin,
    manifest,
};

pub(super) fn validate_options(options: &AcquireOptions) -> Result<(), AcquireErrorKind> {
    for origin in &options.allowed_redirect_origins {
        parse_origin(origin)?;
    }
    for origin in &options.insecure_test_origins {
        let url = parse_origin(origin)?;
        let loopback = match url.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if url.scheme() != "http" || !loopback {
            return Err(AcquireErrorKind::InvalidOptions {
                reason: "HTTP test exceptions require an exact numeric-loopback origin",
            });
        }
    }
    Ok(())
}

fn parse_origin(value: &str) -> Result<Url, AcquireErrorKind> {
    let invalid = || AcquireErrorKind::InvalidOptions {
        reason: "redirect/test allowances must be absolute HTTP(S) origins without paths, queries, or credentials",
    };
    let uri = manifest::parse_uri(value).map_err(|_| invalid())?;
    let url = Url::parse(uri.as_str()).map_err(|_| invalid())?;
    if !uri.is_remote() || url.path() != "/" || url.query().is_some() {
        return Err(invalid());
    }
    Ok(url)
}

fn origin(uri: &Uri) -> Result<String, AcquireErrorKind> {
    let url = Url::parse(uri.as_str()).map_err(|_| AcquireErrorKind::InvalidResponse)?;
    Ok(url.origin().ascii_serialization())
}

fn contains_origin(allowances: &[String], wanted: &str) -> bool {
    allowances.iter().any(|value| {
        parse_origin(value).is_ok_and(|url| url.origin().ascii_serialization() == wanted)
    })
}

fn check_scheme(uri: &Uri, options: &AcquireOptions) -> Result<(), AcquireErrorKind> {
    manifest::parse_uri(uri.as_str())?;
    match uri.scheme() {
        "https" => Ok(()),
        "http" if contains_origin(&options.insecure_test_origins, &origin(uri)?) => Ok(()),
        "http" => Err(AcquireErrorKind::InsecureScheme),
        _ => Err(AcquireErrorKind::UnsupportedScheme),
    }
}

pub(super) fn retrieve(
    resource: &ResourcePin,
    context: &Context,
    budget: &Budget<'_>,
    byte_limit: u64,
) -> Result<(Vec<u8>, Vec<RedirectHop>, usize), AcquireError> {
    let mut current = resource.requested_uri().clone();
    let mut redirects = Vec::<RedirectHop>::new();
    loop {
        let with_ledger = |mut error: AcquireError| {
            error.redirects = redirects.clone();
            error
        };
        budget.check(context).map_err(with_ledger)?;
        check_scheme(&current, budget.options).map_err(|kind| with_ledger(context.error(kind)))?;
        // Reselect on every hop, even within one origin. Header values enter
        // only the child's stdin, never process arguments or a persistent file.
        let request_headers =
            credential_headers(resource.requested_uri(), &current, budget.options)
                .map_err(|kind| with_ledger(context.error(kind)))?;
        budget.check(context).map_err(with_ledger)?;
        let response = request(
            &current,
            resource.media_type(),
            request_headers,
            context,
            budget,
            byte_limit,
        )
        .map_err(with_ledger)?;
        if manifest::is_redirect(response.status) {
            let target = response
                .location
                .as_deref()
                .ok_or_else(|| with_ledger(context.error(AcquireErrorKind::InvalidResponse)))?;
            let next = current
                .join(target)
                .map_err(|_| with_ledger(context.error(AcquireErrorKind::InvalidResponse)))?;
            redirects.push(RedirectHop {
                from_uri: current.clone(),
                to_uri: next.clone(),
                status: response.status,
            });
            let denied = |kind| {
                let mut error = context.error(kind);
                error.redirects = redirects.clone();
                error
            };
            if redirects.len() > budget.options.max_redirects {
                return Err(denied(AcquireErrorKind::TooManyRedirects {
                    limit: budget.options.max_redirects,
                }));
            }
            check_scheme(&next, budget.options).map_err(denied)?;
            let next_origin = origin(&next).map_err(denied)?;
            if origin(&current).map_err(denied)? != next_origin
                && !contains_origin(&budget.options.allowed_redirect_origins, &next_origin)
            {
                return Err(denied(AcquireErrorKind::RedirectDenied));
            }
            if resource.redirects().get(redirects.len() - 1) != redirects.last() {
                return Err(denied(AcquireErrorKind::RedirectDrift));
            }
            current = next;
            continue;
        }
        if response.status != 200 {
            return Err(with_ledger(context.error(AcquireErrorKind::HttpStatus {
                status: response.status,
            })));
        }
        if &current != resource.effective_uri() {
            return Err(with_ledger(
                context.error(AcquireErrorKind::EffectiveUriDrift),
            ));
        }
        if redirects != resource.redirects() {
            return Err(with_ledger(context.error(AcquireErrorKind::RedirectDrift)));
        }
        let attempts = redirects.len() + 1;
        return Ok((response.body, redirects, attempts));
    }
}

fn credential_headers(
    requested: &Uri,
    current: &Uri,
    options: &AcquireOptions,
) -> Result<String, AcquireErrorKind> {
    let credentials = match &options.credentials {
        Some(provider) => provider
            .headers(requested, &origin(current)?)
            .map_err(|_| AcquireErrorKind::Credentials)?,
        None => Vec::new(),
    };
    let mut headers = String::from(
        "Accept: application/json, application/*+json, application/yaml, text/yaml, application/*+yaml\r\nAccept-Encoding: identity\r\nConnection: close\r\n",
    );
    let mut seen = BTreeSet::new();
    // Include the automatic request line, Host and fixed User-Agent in the cap.
    let overhead = current.as_str().len().saturating_add(128);
    if headers.len().saturating_add(overhead) > options.max_request_header_bytes {
        return Err(AcquireErrorKind::HeadersTooLarge {
            limit: options.max_request_header_bytes,
        });
    }
    for credential in credentials {
        let name = credential.name.to_ascii_lowercase();
        if name.is_empty()
            || !name.bytes().all(is_token)
            || !seen.insert(name.clone())
            || name.starts_with("proxy-")
            || matches!(
                name.as_str(),
                "host"
                    | "connection"
                    | "content-length"
                    | "transfer-encoding"
                    | "te"
                    | "trailer"
                    | "upgrade"
                    | "accept"
                    | "accept-encoding"
                    | "user-agent"
                    | "expect"
                    | "referer"
                    | "range"
            )
            || credential
                .value
                .bytes()
                .any(|byte| !(b' '..=b'~').contains(&byte))
        {
            return Err(AcquireErrorKind::InvalidHeader);
        }
        if seen.len().saturating_add(3) > options.max_header_count {
            return Err(AcquireErrorKind::TooManyHeaders {
                limit: options.max_header_count,
            });
        }
        if headers
            .len()
            .saturating_add(name.len())
            .saturating_add(credential.value.len())
            .saturating_add(overhead)
            .saturating_add(4)
            > options.max_request_header_bytes
        {
            return Err(AcquireErrorKind::HeadersTooLarge {
                limit: options.max_request_header_bytes,
            });
        }
        headers.push_str(&name);
        headers.push_str(": ");
        headers.push_str(&credential.value);
        headers.push_str("\r\n");
    }
    Ok(headers)
}

fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

struct Response {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn request(
    uri: &Uri,
    media_type: &str,
    headers: String,
    context: &Context,
    budget: &Budget<'_>,
    byte_limit: u64,
) -> Result<Response, AcquireError> {
    budget.check(context)?;
    let remaining = budget.deadline.saturating_duration_since(Instant::now());
    let seconds = format!("{:.3}", remaining.as_secs_f64().max(0.001));
    let mut command = Command::new(&budget.options.curl_program);
    command
        .env_clear()
        // --disable MUST be the first argument: no .curlrc, even a user home file.
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--globoff",
            "--http1.1",
            "--request",
            "GET",
            "--include",
            "--no-buffer",
            "--no-location",
            "--max-redirs",
            "0",
            "--retry",
            "0",
            "--proxy",
            "",
            "--noproxy",
            "*",
            "--no-netrc",
            "--no-netrc-optional",
            "--no-compressed",
            "--raw",
            "--path-as-is",
            "--disallow-username-in-url",
            "--user-agent",
            "suspect-pins/1",
            "--header",
            "@-",
            "--proto",
        ])
        .arg(if uri.scheme() == "https" {
            "=https"
        } else {
            "=http"
        })
        .arg("--max-time")
        .arg(&seconds)
        .arg("--connect-timeout")
        .arg(&seconds)
        .arg("--max-filesize")
        .arg(
            byte_limit
                .saturating_add(budget.options.max_header_bytes as u64)
                .to_string(),
        )
        .arg("--url")
        .arg(uri.as_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command
        .spawn()
        .map_err(|_| context.error(AcquireErrorKind::TransportUnavailable))?;
    std::thread::scope(|scope| {
        // The guard is dropped before scoped workers are joined, so any early
        // limit/cancellation/error closes the pipes and reaps the child first.
        let mut child = ChildGuard(child);
        let mut stdin = child.0.stdin.take().expect("piped curl stdin");
        let stdout = child.0.stdout.take().expect("piped curl stdout");
        let (send, receive) = mpsc::sync_channel(1);
        scope.spawn(move || {
            let _ = stdin.write_all(headers.as_bytes());
        });
        scope.spawn(move || {
            let _ = send.send(read_response(
                stdout,
                media_type,
                budget.options,
                byte_limit,
            ));
        });
        let mut result = None;
        loop {
            budget.check(context)?;
            if result.is_none() {
                match receive.recv_timeout(Duration::from_millis(5)) {
                    Ok(Ok(response)) if response.status != 200 => return Ok(response),
                    Ok(Err(kind)) if kind != AcquireErrorKind::InvalidResponse => {
                        return Err(context.error(kind));
                    }
                    Ok(value) => result = Some(value),
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        result = Some(Err(AcquireErrorKind::InvalidResponse))
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            if let Some(status) = child
                .0
                .try_wait()
                .map_err(|_| context.error(AcquireErrorKind::Transport { status: None }))?
            {
                // An early parser rejection closes stdout. curl can exit on
                // that closed pipe before the worker publishes its result;
                // preserve the specific policy error rather than racing it
                // with a generic process failure.
                if result.is_none() {
                    continue;
                }
                if !status.success() {
                    let kind = match status.code() {
                        Some(28) => AcquireErrorKind::Timeout,
                        Some(35 | 51 | 58 | 60 | 77 | 82 | 83 | 90 | 91) => AcquireErrorKind::Tls,
                        Some(63) => AcquireErrorKind::TooLarge { limit: byte_limit },
                        _ => AcquireErrorKind::Transport {
                            status: status.code(),
                        },
                    };
                    return Err(context.error(kind));
                }
                if let Some(result) = result {
                    return result.map_err(|kind| context.error(kind));
                }
            }
            if result.is_some() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    })
}

fn read_response(
    reader: impl Read,
    expected_media: &str,
    options: &AcquireOptions,
    byte_limit: u64,
) -> Result<Response, AcquireErrorKind> {
    let mut reader = BufReader::with_capacity(8192, reader);
    let mut header_bytes = 0usize;
    let mut header_count = 0usize;
    let (status, headers) = loop {
        let line = header_line(&mut reader, &mut header_bytes, options.max_header_bytes)?;
        count_header(&mut header_count, options.max_header_count)?;
        let text = std::str::from_utf8(&line).map_err(|_| AcquireErrorKind::InvalidResponse)?;
        let mut parts = text.split_whitespace();
        if !matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
            return Err(AcquireErrorKind::InvalidResponse);
        }
        let status_text = parts.next().ok_or(AcquireErrorKind::InvalidResponse)?;
        if status_text.len() != 3 || !status_text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AcquireErrorKind::InvalidResponse);
        }
        let status = status_text
            .parse::<u16>()
            .map_err(|_| AcquireErrorKind::InvalidResponse)?;
        let mut headers = BTreeMap::new();
        loop {
            let line = header_line(&mut reader, &mut header_bytes, options.max_header_bytes)?;
            if line.is_empty() {
                break;
            }
            count_header(&mut header_count, options.max_header_count)?;
            if line.first().is_some_and(u8::is_ascii_whitespace)
                || line
                    .iter()
                    .any(|byte| byte.is_ascii_control() && *byte != b'\t')
            {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            let text = std::str::from_utf8(&line).map_err(|_| AcquireErrorKind::InvalidResponse)?;
            let (name, value) = text
                .split_once(':')
                .ok_or(AcquireErrorKind::InvalidResponse)?;
            if name.is_empty() || !name.bytes().all(is_token) {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            let name = name.to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "content-type"
                    | "content-encoding"
                    | "content-length"
                    | "transfer-encoding"
                    | "location"
            ) && headers.insert(name, value.trim().to_owned()).is_some()
            {
                return Err(AcquireErrorKind::InvalidResponse);
            }
        }
        if matches!(status, 100..=199) {
            if status == 101 {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            continue;
        }
        break (status, headers);
    };
    let location = headers.get("location").cloned();
    // Redirect/error bodies are never needed and may be arbitrarily large or
    // stalled. Return once bounded headers are known; the owner kills the child.
    if status != 200 {
        return Ok(Response {
            status,
            location,
            body: Vec::new(),
        });
    }
    if headers
        .get("content-encoding")
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
        || headers
            .get("transfer-encoding")
            .is_some_and(|value| !value.eq_ignore_ascii_case("chunked"))
    {
        return Err(AcquireErrorKind::UnsupportedEncoding);
    }
    let media = headers
        .get("content-type")
        .ok_or(AcquireErrorKind::BadMediaType)?;
    if manifest::media_type(media)? != expected_media {
        return Err(AcquireErrorKind::BadMediaType);
    }
    let length = headers
        .get("content-length")
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            value
                .parse::<u64>()
                .map_err(|_| AcquireErrorKind::InvalidResponse)
        })
        .transpose()?;
    if length.is_some() && headers.contains_key("transfer-encoding") {
        return Err(AcquireErrorKind::InvalidResponse);
    }
    if length.is_some_and(|size| size > byte_limit) {
        return Err(AcquireErrorKind::TooLarge { limit: byte_limit });
    }
    if headers.contains_key("transfer-encoding") {
        let body = read_chunked(
            &mut reader,
            byte_limit,
            &mut header_bytes,
            &mut header_count,
            options,
        )?;
        return Ok(Response {
            status,
            location,
            body,
        });
    }
    let mut body = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let available = byte_limit
            .saturating_sub(body.len() as u64)
            .saturating_add(1)
            .min(chunk.len() as u64) as usize;
        let read = reader
            .read(&mut chunk[..available])
            .map_err(|_| AcquireErrorKind::InvalidResponse)?;
        if read == 0 {
            break;
        }
        if (body.len() as u64).saturating_add(read as u64) > byte_limit {
            return Err(AcquireErrorKind::TooLarge { limit: byte_limit });
        }
        body.extend_from_slice(&chunk[..read]);
    }
    if length.is_some_and(|expected| expected != body.len() as u64) {
        return Err(AcquireErrorKind::InvalidResponse);
    }
    Ok(Response {
        status,
        location,
        body,
    })
}

fn read_chunked(
    reader: &mut impl BufRead,
    byte_limit: u64,
    header_bytes: &mut usize,
    header_count: &mut usize,
    options: &AcquireOptions,
) -> Result<Vec<u8>, AcquireErrorKind> {
    // curl --raw leaves transfer framing visible. Charge chunk extensions,
    // delimiters and trailers to the header budget, bounding wire metadata
    // independently of the decoded document's size.
    let mut body = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let line = header_line(reader, header_bytes, options.max_header_bytes)?;
        if line.iter().any(|byte| !(b' '..=b'~').contains(byte)) {
            return Err(AcquireErrorKind::InvalidResponse);
        }
        let size = line.split(|byte| *byte == b';').next().unwrap_or_default();
        if size.is_empty() || size.len() > 16 || !size.iter().all(u8::is_ascii_hexdigit) {
            return Err(AcquireErrorKind::InvalidResponse);
        }
        let mut remaining = u64::from_str_radix(
            std::str::from_utf8(size).map_err(|_| AcquireErrorKind::InvalidResponse)?,
            16,
        )
        .map_err(|_| AcquireErrorKind::InvalidResponse)?;
        if remaining > byte_limit.saturating_sub(body.len() as u64) {
            return Err(AcquireErrorKind::TooLarge { limit: byte_limit });
        }
        if remaining == 0 {
            loop {
                let trailer = header_line(reader, header_bytes, options.max_header_bytes)?;
                if trailer.is_empty() {
                    break;
                }
                count_header(header_count, options.max_header_count)?;
                let value =
                    std::str::from_utf8(&trailer).map_err(|_| AcquireErrorKind::InvalidResponse)?;
                let (name, value) = value
                    .split_once(':')
                    .ok_or(AcquireErrorKind::InvalidResponse)?;
                if name.is_empty()
                    || !name.bytes().all(is_token)
                    || value
                        .bytes()
                        .any(|byte| byte.is_ascii_control() && byte != b'\t')
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        "content-type"
                            | "content-encoding"
                            | "content-length"
                            | "transfer-encoding"
                            | "location"
                    )
                {
                    return Err(AcquireErrorKind::InvalidResponse);
                }
            }
            if reader
                .read(&mut chunk[..1])
                .map_err(|_| AcquireErrorKind::InvalidResponse)?
                != 0
            {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            return Ok(body);
        }
        while remaining > 0 {
            let count = remaining.min(chunk.len() as u64) as usize;
            let count = reader
                .read(&mut chunk[..count])
                .map_err(|_| AcquireErrorKind::InvalidResponse)?;
            if count == 0 {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            body.extend_from_slice(&chunk[..count]);
            remaining -= count as u64;
        }
        if !header_line(reader, header_bytes, options.max_header_bytes)?.is_empty() {
            return Err(AcquireErrorKind::InvalidResponse);
        }
    }
}

fn count_header(count: &mut usize, limit: usize) -> Result<(), AcquireErrorKind> {
    *count = count.saturating_add(1);
    if *count > limit {
        return Err(AcquireErrorKind::TooManyHeaders { limit });
    }
    Ok(())
}

fn header_line(
    reader: &mut impl BufRead,
    used: &mut usize,
    limit: usize,
) -> Result<Vec<u8>, AcquireErrorKind> {
    let mut line = Vec::new();
    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|_| AcquireErrorKind::InvalidResponse)?;
        if buffer.is_empty() {
            return Err(AcquireErrorKind::InvalidResponse);
        }
        let end = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let count = end.unwrap_or(buffer.len());
        if count > limit.saturating_sub(*used) {
            return Err(AcquireErrorKind::HeadersTooLarge { limit });
        }
        *used += count;
        line.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if end.is_some() {
            if !line.ends_with(b"\r\n") {
                return Err(AcquireErrorKind::InvalidResponse);
            }
            line.truncate(line.len() - 2);
            return Ok(line);
        }
    }
}
