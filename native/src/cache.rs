use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::Diagnostic;

const COMPRESSION_THRESHOLD: usize = 4 * 1024;
// Cap allocations from compressed frame headers. Larger records are stored as plain JSON.
const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default)]
pub struct CacheRead {
    pub diagnostic: Option<Diagnostic>,
    pub value: Option<Value>,
}

pub fn key(
    config_json: &str,
    file: &str,
    source: &str,
    modified_at: Option<&str>,
    target_mode: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in [
        "amamo-mdx-cache-v3",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        target_mode,
        config_json,
        file,
        source,
        modified_at.unwrap_or(""),
    ] {
        hasher.update(value.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

pub fn read(directory: &Path, key: &str) -> Result<CacheRead, Vec<Diagnostic>> {
    let path = entry_path(directory, key)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CacheRead::default());
        }
        Err(error) => {
            return Err(vec![Diagnostic::error(
                "AMAMO_CACHE_READ",
                path.to_str(),
                format!("Could not read cache entry: {error}"),
            )]);
        }
    };
    match decode(&bytes) {
        Ok(value) => Ok(CacheRead {
            diagnostic: None,
            value: Some(value),
        }),
        Err(error) => {
            remove_if_present(&path)
                .map_err(|error| io_diagnostic("AMAMO_CACHE_READ", &path, error))?;
            Ok(CacheRead {
                diagnostic: Some(Diagnostic::warning(
                    "AMAMO_CACHE_CORRUPT",
                    path.to_str(),
                    format!("Removed corrupt cache entry: {error}"),
                )),
                value: None,
            })
        }
    }
}

fn decode(bytes: &[u8]) -> io::Result<Value> {
    let decoded;
    let json = if bytes.starts_with(&zstd::zstd_safe::MAGICNUMBER.to_le_bytes()) {
        let size = zstd::zstd_safe::get_frame_content_size(bytes)
            .ok()
            .flatten()
            .and_then(|size| usize::try_from(size).ok())
            .filter(|&size| size <= MAX_DECOMPRESSED_BYTES)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid cache frame size")
            })?;
        decoded = zstd::bulk::decompress(bytes, size)?;
        decoded.as_slice()
    } else {
        bytes
    };
    Ok(serde_json::from_slice(json)?)
}

pub fn write(
    directory: &Path,
    key: &str,
    value: &impl serde::Serialize,
) -> Result<bool, Vec<Diagnostic>> {
    let path = entry_path(directory, key)?;
    let mut bytes = serde_json::to_vec(value).map_err(|error| {
        vec![Diagnostic::error(
            "AMAMO_CACHE_WRITE",
            path.to_str(),
            format!("Could not encode cache entry: {error}"),
        )]
    })?;
    if (COMPRESSION_THRESHOLD..=MAX_DECOMPRESSED_BYTES).contains(&bytes.len()) {
        let compressed = zstd::bulk::compress(&bytes, 1)
            .map_err(|error| io_diagnostic("AMAMO_CACHE_WRITE", &path, error))?;
        if compressed.len() < bytes.len() {
            bytes = compressed;
        }
    }
    if fs::read(&path).is_ok_and(|existing| existing == bytes) {
        return Ok(false);
    }
    let parent = path.parent().expect("cache entry has a parent");
    fs::create_dir_all(parent)
        .map_err(|error| io_diagnostic("AMAMO_CACHE_WRITE", parent, error))?;
    let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{key}.{}.{}.tmp", std::process::id(), id));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_diagnostic("AMAMO_CACHE_WRITE", &temporary, error))?;
    file.write_all(&bytes)
        .map_err(|error| io_diagnostic("AMAMO_CACHE_WRITE", &temporary, error))?;
    drop(file);
    if let Err(error) = fs::rename(&temporary, &path) {
        if path.exists() {
            fs::remove_file(&temporary).map_err(|remove_error| {
                io_diagnostic("AMAMO_CACHE_WRITE", &temporary, remove_error)
            })?;
        } else {
            return Err(io_diagnostic("AMAMO_CACHE_WRITE", &path, error));
        }
    }
    Ok(true)
}

pub fn prune(directory: &Path, keep: &HashSet<String>) -> Result<usize, Vec<Diagnostic>> {
    let directories = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_diagnostic("AMAMO_CACHE_PRUNE", directory, error)),
    };
    let mut removed = 0;
    for entry in directories {
        let entry = entry.map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", directory, error))?;
        if !entry
            .file_type()
            .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &entry.path(), error))?
            .is_dir()
        {
            continue;
        }
        let child_directory = entry.path();
        let children = match fs::read_dir(&child_directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_diagnostic("AMAMO_CACHE_PRUNE", &child_directory, error)),
        };
        for child in children {
            let child = child
                .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &child_directory, error))?;
            let name = child.file_name();
            let name = name.to_string_lossy();
            let Some(key) = name
                .strip_suffix(".cache")
                .or_else(|| name.strip_suffix(".json").filter(|key| key.len() == 64))
            else {
                continue;
            };
            if valid_key(key) && !keep.contains(key) {
                let path = child.path();
                let deleted = remove_if_present(&path)
                    .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &path, error))?;
                removed += usize::from(deleted);
            }
        }
        if fs::read_dir(&child_directory).is_ok_and(|mut entries| entries.next().is_none())
            && let Err(error) = fs::remove_dir(&child_directory)
            && !matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            )
        {
            return Err(io_diagnostic("AMAMO_CACHE_PRUNE", &child_directory, error));
        }
    }
    Ok(removed)
}

pub fn remove(directory: &Path, key: &str) -> Result<(), Vec<Diagnostic>> {
    let path = entry_path(directory, key)?;
    remove_if_present(&path)
        .map(|_| ())
        .map_err(|error| io_diagnostic("AMAMO_CACHE_WRITE", &path, error))
}

fn remove_if_present(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn entry_path(directory: &Path, key: &str) -> Result<PathBuf, Vec<Diagnostic>> {
    if !valid_key(key) {
        return Err(vec![Diagnostic::error(
            "AMAMO_CACHE_KEY_INVALID",
            None,
            "Cache keys must contain at least two hexadecimal characters",
        )]);
    }
    Ok(directory.join(&key[..2]).join(format!("{key}.cache")))
}

fn valid_key(key: &str) -> bool {
    key.len() >= 2 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn io_diagnostic(code: &str, path: &Path, error: std::io::Error) -> Vec<Diagnostic> {
    vec![Diagnostic::error(code, path.to_str(), error.to_string())]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use super::{entry_path, prune, read, write};

    #[test]
    fn compact_records_round_trip_and_truncated_frames_become_cache_misses() {
        let directory =
            std::env::temp_dir().join(format!("amamo-mdx-cache-roundtrip-{}", std::process::id()));
        let entry = entry_path(&directory, "abcdef").unwrap();
        let small = serde_json::json!({"module": "hello"});
        let large = serde_json::json!({"module": "highlighted code ".repeat(1024)});
        write(&directory, "abcdef", &small).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&entry).unwrap()).unwrap(),
            small
        );

        write(&directory, "abcdef", &large).unwrap();
        let compressed = fs::read(&entry).unwrap();
        assert!(compressed.len() < 4096);
        assert_eq!(read(&directory, "abcdef").unwrap().value, Some(large));
        fs::write(&entry, &compressed[..compressed.len() - 1]).unwrap();
        let result = read(&directory, "abcdef").unwrap();
        assert!(result.value.is_none());
        assert_eq!(result.diagnostic.unwrap().code, "AMAMO_CACHE_CORRUPT");
        assert!(!entry.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn full_prune_collects_previous_format_records() {
        let directory =
            std::env::temp_dir().join(format!("amamo-mdx-cache-upgrade-{}", std::process::id()));
        let key = "ab".repeat(32);
        let legacy = directory.join("ab").join(format!("{key}.json"));
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"{}").unwrap();
        assert_eq!(prune(&directory, &HashSet::new()).unwrap(), 1);
        assert!(!legacy.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn corrupt_cache_is_removed_and_becomes_a_miss() {
        let directory =
            std::env::temp_dir().join(format!("amamo-mdx-cache-{}", std::process::id()));
        let entry_directory = directory.join("ab");
        let entry = entry_directory.join("abcdef.cache");
        fs::create_dir_all(&entry_directory).unwrap();
        fs::write(&entry, b"{truncated").unwrap();

        let result = read(&directory, "abcdef").unwrap();

        assert!(result.value.is_none());
        assert_eq!(result.diagnostic.unwrap().code, "AMAMO_CACHE_CORRUPT");
        assert!(!entry.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
