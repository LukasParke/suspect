use super::parameters::Writer;
use super::{
    ClientOptions, Operation, SdkError, Server, Source, representation_error, resource_error,
};
use url::Url;

pub(super) fn resolve(op: &Operation, options: &ClientOptions) -> Result<Url, SdkError> {
    if let Some(value) = &options.server_url {
        if options.server_index.is_some() || !options.server_variables.is_empty() {
            return Err(representation_error(
                op.source,
                op.source,
                "absolute server override cannot also select a declared candidate or variables",
            ));
        }
        return validate(
            op.source,
            op.source,
            value,
            !options.allow_insecure_http,
            op.limits.request,
        );
    }
    let server = op
        .servers
        .get(options.server_index.unwrap_or(0))
        .ok_or_else(|| {
            representation_error(
                op.source,
                op.source,
                "server candidate index is not declared",
            )
        })?;
    let value = expand(op, server, options)?;
    match Url::parse(&value) {
        Ok(_) => validate(op.source, server.source, &value, false, op.limits.request),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            let document = options
                .document_url
                .as_deref()
                .unwrap_or(server.document_base.document);
            check_encoded_dot_segments(op.source, server.document_base, document)?;
            let base = Url::parse(document).map_err(|_| {
                representation_error(
                    op.source,
                    server.source,
                    "relative server requires an absolute HTTP document URL",
                )
            })?;
            if !matches!(base.scheme(), "http" | "https")
                || !base.username().is_empty()
                || base.password().is_some()
            {
                return Err(representation_error(
                    op.source,
                    server.source,
                    "local descriptions need an explicit HTTP document URL for relative servers",
                ));
            }
            let resolved = base.join(&value).map_err(|_| {
                representation_error(
                    op.source,
                    server.source,
                    "relative server URL could not be resolved",
                )
            })?;
            validate(op.source, server.source, resolved.as_str(), false, op.limits.request)
        }
        _ => Err(representation_error(
            op.source,
            server.source,
            "server URL is invalid",
        )),
    }
}
fn expand(op: &Operation, server: &Server, options: &ClientOptions) -> Result<String, SdkError> {
    for name in options.server_variables.keys() {
        if !server.variables.iter().any(|v| v.name == name) {
            return Err(representation_error(
                op.source,
                server.source,
                "undeclared server variable override",
            ));
        }
    }
    for variable in server.variables {
        if let Some(value) = options.server_variables.get(variable.name) {
            if !variable.values.is_empty() && !variable.values.iter().any(|v| v.value == value) {
                return Err(representation_error(
                    op.source,
                    variable.source,
                    "server variable override is outside its enum",
                ));
            }
        }
    }
    let mut out = Writer::new(op.source, server.source, op.limits.request);
    let mut rest = server.template;
    while let Some(open) = rest.find('{') {
        out.push(&rest[..open])?;
        let tail = &rest[open + 1..];
        let close = tail.find('}').ok_or_else(|| {
            representation_error(op.source, server.source, "invalid server template")
        })?;
        let variable = server
            .variables
            .iter()
            .find(|v| v.name == &tail[..close])
            .ok_or_else(|| {
                representation_error(
                    op.source,
                    server.source,
                    "undeclared server template variable",
                )
            })?;
        out.push(
            options
                .server_variables
                .get(variable.name)
                .map(String::as_str)
                .unwrap_or(variable.default.value),
        )?;
        rest = &tail[close + 1..];
    }
    out.push(rest)?;
    if out.value.contains(['?', '#', '\\', '{', '}'])
        || out
            .value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || !percent_triples(&out.value)
    {
        return Err(representation_error(
            op.source,
            server.source,
            "server URL contains an invalid or route-changing value",
        ));
    }
    check_encoded_dot_segments(op.source, server.source, &out.value)?;
    Ok(out.value)
}
// RFC3986 does not treat encoded dots as path-navigation operators, and ADR 0035
// keeps them as opaque path data. The pinned WHATWG URL/reqwest stack decodes
// and resolves them, so this one representation is refused as unsendable
// before it can join or normalize the route. Encoded slashes and other encoded
// octets are preserved verbatim.
fn check_encoded_dot_segments(
    operation: Source,
    source: Source,
    value: &str,
) -> Result<(), SdkError> {
    let route = value.split(['?', '#']).next().unwrap_or(value);
    if route
        .split('/')
        .any(|segment| segment.contains('%') && dot_segment(segment))
    {
        return Err(representation_error(
            operation,
            source,
            "encoded dot path segments cannot be preserved by the native URL transport",
        ));
    }
    Ok(())
}
/// One server URL policy (ADR 0035): a declared server keeps its scheme; a
/// caller override (`loopback_only`) is https, or plain http only to loopback
/// unless `allow_insecure_http` is set; no userinfo, query or fragment; a
/// valid port; literal `.`/`..` segments refused, never resolved; non-ASCII,
/// controls, backslashes and malformed escapes refused.
fn validate(
    operation: Source,
    source: Source,
    value: &str,
    loopback_only: bool,
    limit: usize,
) -> Result<Url, SdkError> {
    if value.len() > limit {
        return Err(resource_error(
            operation,
            source,
            "server URL exceeds the request byte ceiling",
        ));
    }
    if value.is_empty()
        || !value.is_ascii()
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains(['\\', '{', '}'])
        || !percent_triples(value)
    {
        return Err(representation_error(
            operation,
            source,
            "invalid server URL characters",
        ));
    }
    check_encoded_dot_segments(operation, source, value)?;
    let url = Url::parse(value).map_err(|_| {
        representation_error(operation, source, "server must be an absolute HTTP URL")
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(representation_error(
            operation,
            source,
            "server URL requires HTTP(S), a host, a valid port, and no credentials, query or fragment",
        ));
    }
    if loopback_only
        && url.scheme() == "http"
        && !match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
            Some(url::Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
            _ => false,
        }
    {
        return Err(representation_error(
            operation,
            source,
            "a plain HTTP server override must target loopback unless allow_insecure_http is set",
        ));
    }
    let after_authority = value.find("://").map_or(0, |at| at + 3);
    if value[after_authority..]
        .find('/')
        .is_some_and(|at| value[after_authority + at..].split('/').any(dot_segment))
    {
        return Err(representation_error(
            operation,
            source,
            "absolute server path contains a dot segment",
        ));
    }
    Ok(url)
}
fn dot_segment(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "." | ".." | "%2e" | ".%2e" | "%2e." | "%2e%2e"
    )
}
fn percent_triples(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            if !bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit)
            {
                return false;
            }
            at += 3;
        } else {
            at += 1;
        }
    }
    true
}
pub(super) fn assemble(
    op: &Operation,
    base: &Url,
    path: &str,
    query: &[String],
) -> Result<String, SdkError> {
    if path.contains(['{', '}', '\\', '?', '#']) || path.split('/').any(dot_segment) {
        return Err(representation_error(
            op.source,
            op.source,
            "operation path has an unbound or route-normalizing value",
        ));
    }
    let mut out = Writer::new(op.source, op.source, op.limits.request);
    out.push(&base.as_str()[..base.as_str().len() - base.path().len()])?;
    out.push(base.path().strip_suffix('/').unwrap_or(base.path()))?;
    out.push("/")?;
    out.push(path.strip_prefix('/').unwrap_or(path))?;
    for (i, value) in query.iter().enumerate() {
        out.push(if i == 0 { "?" } else { "&" })?;
        out.push(value)?;
    }
    if Url::parse(&out.value).is_ok_and(|url| url.as_str() == out.value) {
        Ok(out.value)
    } else {
        Err(representation_error(
            op.source,
            op.source,
            "operation URL would be normalized to a different wire value",
        ))
    }
}
