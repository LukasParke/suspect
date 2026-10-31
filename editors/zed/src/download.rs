//! The pure half of the managed download: which release asset belongs to
//! this platform, checking it against its `.sha256`, and taking the
//! `suspect` executable out of the archive. Nothing here calls Zed, so it
//! is tested on the host (`cargo test`); `lib.rs` does the fetching.
//!
//! The asset contract is P5's (docs/proposals/P5-release.md, "Asset
//! contract"): release `v{V}` carries `suspect-cli-{TRIPLE}.tar.gz` (`.zip`
//! on Windows) and `{archive}.sha256`, whose first token is the lowercase
//! hex SHA-256; the binary sits at the archive root or one directory down.

use std::io::Read;

use sha2::{Digest, Sha256};
use zed_extension_api::{Architecture, Os};

/// The release triple for a platform; `None` where no build is published
/// (the caller falls back to `PATH`). Linux uses the static musl build.
#[must_use]
pub fn triple(os: Os, arch: Architecture) -> Option<&'static str> {
    match (os, arch) {
        (Os::Mac, Architecture::Aarch64) => Some("aarch64-apple-darwin"),
        (Os::Mac, Architecture::X8664) => Some("x86_64-apple-darwin"),
        (Os::Linux, Architecture::X8664) => Some("x86_64-unknown-linux-musl"),
        (Os::Linux, Architecture::Aarch64) => Some("aarch64-unknown-linux-musl"),
        (Os::Windows, Architecture::X8664) => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// The executable's file name.
#[must_use]
pub fn exe_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "suspect.exe",
        _ => "suspect",
    }
}

/// The archive kind an asset name implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archive {
    /// `.tar.gz`
    TarGz,
    /// `.zip`
    Zip,
}

/// The archive asset for a triple: `suspect-cli-{triple}.tar.gz`, `.zip`
/// for Windows.
#[must_use]
pub fn asset_name(triple: &str) -> (String, Archive) {
    if triple.contains("windows") {
        (format!("suspect-cli-{triple}.zip"), Archive::Zip)
    } else {
        (format!("suspect-cli-{triple}.tar.gz"), Archive::TarGz)
    }
}

/// The digest a `.sha256` file names: its first whitespace-separated
/// token, which must be 64 hex digits.
///
/// # Errors
/// When the file does not start with a SHA-256 in hex.
pub fn parse_sha256(text: &str) -> Result<String, String> {
    let token = text.split_whitespace().next().unwrap_or_default();
    if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(token.to_ascii_lowercase())
    } else {
        Err(format!(
            "not a SHA-256 checksum file: {:?}",
            text.lines().next().unwrap_or_default()
        ))
    }
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Checks `bytes` against the expected lowercase hex digest.
///
/// # Errors
/// On a mismatch, naming both digests.
pub fn verify_sha256(bytes: &[u8], want: &str) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if got.eq_ignore_ascii_case(want.trim()) {
        Ok(())
    } else {
        Err(format!("checksum mismatch: expected {want}, got {got}"))
    }
}

/// Whether an archive member is the executable: named `exe` at the root or
/// one directory down (rule 5 of the asset contract).
fn is_binary(path: &str, exe: &str) -> bool {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    matches!(parts.as_slice(), [name] | [_, name] if *name == exe)
}

/// The executable's bytes from an archive.
///
/// # Errors
/// When the archive does not parse or holds no `exe` at depth 1 or 2.
pub fn extract_binary(bytes: &[u8], kind: Archive, exe: &str) -> Result<Vec<u8>, String> {
    match kind {
        Archive::TarGz => extract_tar_gz(bytes, exe),
        Archive::Zip => extract_zip(bytes, exe),
    }
}

fn extract_tar_gz(bytes: &[u8], exe: &str) -> Result<Vec<u8>, String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let entries = archive
        .entries()
        .map_err(|e| format!("not a .tar.gz: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("bad .tar.gz entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("bad .tar.gz path: {e}"))?
            .to_string_lossy()
            .into_owned();
        if is_binary(&path, exe) {
            let mut out = Vec::new();
            entry
                .read_to_end(&mut out)
                .map_err(|e| format!("cannot read {path}: {e}"))?;
            return Ok(out);
        }
    }
    Err(format!("the archive has no {exe} at depth 1 or 2"))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    bytes
        .get(at..at + 2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(|| "truncated .zip".to_owned())
}

fn u32_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(|| "truncated .zip".to_owned())
}

/// A minimal `.zip` reader: the central directory, then the one member
/// that is the executable, stored or deflated. No zip64 (the archive is a
/// few tens of MB).
fn extract_zip(bytes: &[u8], exe: &str) -> Result<Vec<u8>, String> {
    const EOCD: &[u8] = b"PK\x05\x06";
    const CENTRAL: &[u8] = b"PK\x01\x02";
    const LOCAL: &[u8] = b"PK\x03\x04";
    // The end-of-central-directory record is the last 22 bytes plus an
    // optional comment of up to 64 KiB.
    let floor = bytes.len().saturating_sub(22 + 0xFFFF);
    let eocd = (floor..bytes.len().saturating_sub(21))
        .rev()
        .find(|&i| bytes.get(i..i + 4) == Some(EOCD))
        .ok_or("not a .zip (no end of central directory)")?;
    let count = u16_at(bytes, eocd + 10)?;
    let mut at = u32_at(bytes, eocd + 16)?;
    for _ in 0..count {
        if bytes.get(at..at + 4) != Some(CENTRAL) {
            return Err("bad .zip central directory".into());
        }
        let method = u16_at(bytes, at + 10)?;
        let compressed = u32_at(bytes, at + 20)?;
        let size = u32_at(bytes, at + 24)?;
        let name_len = u16_at(bytes, at + 28)?;
        let extra_len = u16_at(bytes, at + 30)?;
        let comment_len = u16_at(bytes, at + 32)?;
        let local = u32_at(bytes, at + 42)?;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .ok_or("truncated .zip")?;
        let name = String::from_utf8_lossy(name);
        at += 46 + name_len + extra_len + comment_len;
        if !is_binary(&name, exe) {
            continue;
        }
        if bytes.get(local..local + 4) != Some(LOCAL) {
            return Err(format!("bad .zip local header for {name}"));
        }
        let start = local + 30 + u16_at(bytes, local + 26)? + u16_at(bytes, local + 28)?;
        let data = bytes
            .get(start..start + compressed)
            .ok_or("truncated .zip member")?;
        let out = match method {
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(data)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("cannot inflate {name}: {e}"))?;
                out
            }
            other => {
                return Err(format!(
                    "{name}: unsupported .zip compression method {other}"
                ))
            }
        };
        if out.len() != size {
            return Err(format!("{name}: expected {size} bytes, got {}", out.len()));
        }
        return Ok(out);
    }
    Err(format!("the archive has no {exe} at depth 1 or 2"))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn every_published_platform_has_a_triple_and_the_rest_fall_through() {
        assert_eq!(
            triple(Os::Mac, Architecture::Aarch64),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(
            triple(Os::Mac, Architecture::X8664),
            Some("x86_64-apple-darwin")
        );
        assert_eq!(
            triple(Os::Linux, Architecture::X8664),
            Some("x86_64-unknown-linux-musl")
        );
        assert_eq!(
            triple(Os::Linux, Architecture::Aarch64),
            Some("aarch64-unknown-linux-musl")
        );
        assert_eq!(
            triple(Os::Windows, Architecture::X8664),
            Some("x86_64-pc-windows-msvc")
        );
        assert_eq!(triple(Os::Windows, Architecture::Aarch64), None);
        assert_eq!(triple(Os::Linux, Architecture::X86), None);
        assert_eq!(
            asset_name("x86_64-unknown-linux-musl"),
            (
                "suspect-cli-x86_64-unknown-linux-musl.tar.gz".into(),
                Archive::TarGz
            )
        );
        assert_eq!(
            asset_name("x86_64-pc-windows-msvc"),
            (
                "suspect-cli-x86_64-pc-windows-msvc.zip".into(),
                Archive::Zip
            )
        );
        assert_eq!(exe_name(Os::Windows), "suspect.exe");
        assert_eq!(exe_name(Os::Linux), "suspect");
    }

    #[test]
    fn checksums_are_the_first_token_and_must_match() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let bytes = b"suspect";
        let sum = sha256_hex(bytes);
        let file = format!(
            "{}  suspect-cli-x86_64-unknown-linux-musl.tar.gz\n",
            sum.to_uppercase()
        );
        assert_eq!(parse_sha256(&file).unwrap(), sum);
        assert!(verify_sha256(bytes, &sum).is_ok());
        let err = verify_sha256(b"tampered", &sum).unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(parse_sha256("Not Found").is_err());
        assert!(parse_sha256("").is_err());
    }

    fn tar_gz(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, data) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// A `.zip` with one member per entry, stored (`0`) or deflated (`8`).
    fn zip(members: &[(&str, &[u8], u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, method) in members {
            let body = if *method == 8 {
                let mut enc =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                enc.write_all(data).unwrap();
                enc.finish().unwrap()
            } else {
                data.to_vec()
            };
            let offset = out.len() as u32;
            let fields = |sig: &[u8], central: bool| {
                let mut h = sig.to_vec();
                if central {
                    h.extend(20u16.to_le_bytes()); // made by
                }
                h.extend(20u16.to_le_bytes()); // needed
                h.extend(0u16.to_le_bytes()); // flags
                h.extend(method.to_le_bytes());
                h.extend([0u8; 4]); // time, date
                h.extend(0u32.to_le_bytes()); // crc (not checked)
                h.extend((body.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend(0u16.to_le_bytes()); // extra
                if central {
                    h.extend(0u16.to_le_bytes()); // comment
                    h.extend([0u8; 8]); // disk, internal, external attrs
                    h.extend(offset.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            out.extend(fields(b"PK\x03\x04", false));
            out.extend(&body);
            central.extend(fields(b"PK\x01\x02", true));
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(b"PK\x05\x06");
        out.extend([0u8; 4]);
        out.extend((members.len() as u16).to_le_bytes());
        out.extend((members.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn the_binary_is_found_at_the_root_or_one_directory_down() {
        let root = tar_gz(&[("LICENSE-MIT", b"mit"), ("suspect", b"\x7fELF root")]);
        assert_eq!(
            extract_binary(&root, Archive::TarGz, "suspect").unwrap(),
            b"\x7fELF root"
        );
        let nested = tar_gz(&[
            ("suspect-cli-x86_64-unknown-linux-musl/README.md", b"readme"),
            (
                "suspect-cli-x86_64-unknown-linux-musl/suspect",
                b"\x7fELF nested",
            ),
        ]);
        assert_eq!(
            extract_binary(&nested, Archive::TarGz, "suspect").unwrap(),
            b"\x7fELF nested"
        );
        let deep = tar_gz(&[("a/b/suspect", b"too deep")]);
        assert!(extract_binary(&deep, Archive::TarGz, "suspect").is_err());
        assert!(extract_binary(b"not gzip", Archive::TarGz, "suspect").is_err());
    }

    #[test]
    fn zip_members_are_read_stored_or_deflated() {
        let payload = b"MZ windows binary".repeat(50);
        for method in [0u16, 8] {
            let archive = zip(&[
                (
                    "suspect-cli-x86_64-pc-windows-msvc/LICENSE-MIT",
                    b"mit",
                    method,
                ),
                (
                    "suspect-cli-x86_64-pc-windows-msvc/suspect.exe",
                    &payload,
                    method,
                ),
            ]);
            assert_eq!(
                extract_binary(&archive, Archive::Zip, "suspect.exe").unwrap(),
                payload
            );
            assert!(extract_binary(&archive, Archive::Zip, "suspect").is_err());
        }
        assert!(extract_binary(b"PK not really", Archive::Zip, "suspect.exe").is_err());
    }
}
