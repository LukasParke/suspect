use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{AcquireError, AcquireErrorKind, Budget, Context, ResourcePin};
use crate::sha256_digest;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn document_path(base: &Path, resource: &ResourcePin) -> PathBuf {
    let uri_hash = sha256_digest(resource.requested_uri().as_str().as_bytes());
    base.join(&resource.digest()[7..])
        .join(format!("{}.blob", &uri_hash[7..]))
}

pub(super) fn read_verified(
    context: &Context,
    path: &Path,
    digest: &str,
    limit: u64,
    budget: &Budget<'_>,
) -> Result<Option<Vec<u8>>, AcquireError> {
    budget.check(context)?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(context.io("inspecting pinned cache", &error)),
        Ok(metadata) if !metadata.is_file() => {
            return Err(context.error(AcquireErrorKind::Io {
                operation: "reading non-regular pinned cache entry",
                kind: std::io::ErrorKind::InvalidData,
            }));
        }
        Ok(_) => {}
    }
    let bytes = read_source(context, path, limit, budget)?;
    let actual = sha256_digest(&bytes);
    if actual != digest {
        return Err(context.error(AcquireErrorKind::DigestMismatch {
            expected: digest.into(),
            actual,
        }));
    }
    budget.check(context)?;
    Ok(Some(bytes))
}

pub(super) fn read_source(
    context: &Context,
    path: &Path,
    limit: u64,
    budget: &Budget<'_>,
) -> Result<Vec<u8>, AcquireError> {
    budget.check(context)?;
    // Refuse directories/devices/FIFOs before opening, then verify the handle too.
    let metadata = fs::metadata(path).map_err(|error| context.io("inspecting source", &error))?;
    if !metadata.is_file() {
        return Err(context.error(AcquireErrorKind::Io {
            operation: "reading non-regular source",
            kind: std::io::ErrorKind::InvalidData,
        }));
    }
    if metadata.len() > limit {
        return Err(context.error(AcquireErrorKind::TooLarge { limit }));
    }
    budget.check(context)?;
    let mut file = File::open(path).map_err(|error| context.io("opening source", &error))?;
    if !file
        .metadata()
        .map_err(|error| context.io("inspecting open source", &error))?
        .is_file()
    {
        return Err(context.error(AcquireErrorKind::Io {
            operation: "reading non-regular source",
            kind: std::io::ErrorKind::InvalidData,
        }));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        budget.check(context)?;
        let available = limit
            .saturating_sub(bytes.len() as u64)
            .saturating_add(1)
            .min(chunk.len() as u64) as usize;
        let read = file
            .read(&mut chunk[..available])
            .map_err(|error| context.io("reading source", &error))?;
        if read == 0 {
            break;
        }
        if (bytes.len() as u64).saturating_add(read as u64) > limit {
            return Err(context.error(AcquireErrorKind::TooLarge { limit }));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    budget.check(context)?;
    Ok(bytes)
}

struct TempPath(PathBuf);
impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn write_immutable(
    context: &Context,
    path: &Path,
    bytes: &[u8],
    budget: &Budget<'_>,
) -> Result<(), AcquireError> {
    budget.check(context)?;
    let digest = sha256_digest(bytes);
    if read_verified(context, path, &digest, bytes.len() as u64, budget)?.is_some() {
        return Ok(());
    }
    let parent = path.parent().expect("content-addressed cache parent");
    fs::create_dir_all(parent)
        .map_err(|error| context.io("creating pinned cache directory", &error))?;
    budget.check(context)?;
    let mut temporary = None;
    for _ in 0..16 {
        let candidate = parent.join(format!(
            ".pin-{}-{}.part",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((TempPath(candidate), file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(context.io("creating immutable cache file", &error)),
        }
    }
    let (temporary, mut file) = temporary.ok_or_else(|| {
        context.error(AcquireErrorKind::Io {
            operation: "allocating immutable cache file",
            kind: std::io::ErrorKind::AlreadyExists,
        })
    })?;
    for chunk in bytes.chunks(8192) {
        budget.check(context)?;
        file.write_all(chunk)
            .map_err(|error| context.io("writing immutable cache file", &error))?;
    }
    let mut permissions = file
        .metadata()
        .map_err(|error| context.io("inspecting immutable cache file", &error))?
        .permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)
        .map_err(|error| context.io("sealing immutable cache file", &error))?;
    file.sync_all()
        .map_err(|error| context.io("syncing immutable cache file", &error))?;
    drop(file);
    budget.check(context)?;
    // A same-directory hard link installs atomically without replacing even a
    // raced or malicious existing path. Existing bytes must verify independently.
    match fs::hard_link(&temporary.0, path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_verified(context, path, &digest, bytes.len() as u64, budget)?.is_none() {
                return Err(context.error(AcquireErrorKind::CacheMiss));
            }
        }
        Err(error) => return Err(context.io("publishing immutable cache file", &error)),
    }
    Ok(())
}
