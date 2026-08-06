use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::Diagnostic;

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
        "amamo-mdx-cache-v1",
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
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(CacheRead {
            diagnostic: None,
            value: Some(value),
        }),
        Err(error) => {
            fs::remove_file(&path).map_err(|remove_error| {
                vec![Diagnostic::error(
                    "AMAMO_CACHE_READ",
                    path.to_str(),
                    format!("Could not remove corrupt cache entry: {remove_error}"),
                )]
            })?;
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

pub fn write(directory: &Path, key: &str, value: &Value) -> Result<bool, Vec<Diagnostic>> {
    let path = entry_path(directory, key)?;
    let bytes = serde_json::to_vec(value).map_err(|error| {
        vec![Diagnostic::error(
            "AMAMO_CACHE_WRITE",
            path.to_str(),
            format!("Could not encode cache entry: {error}"),
        )]
    })?;
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
        .and_then(|()| file.sync_all())
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
        for child in fs::read_dir(&child_directory)
            .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &child_directory, error))?
        {
            let child = child
                .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &child_directory, error))?;
            let name = child.file_name();
            let name = name.to_string_lossy();
            let Some(key) = name.strip_suffix(".json") else {
                continue;
            };
            if valid_key(key) && !keep.contains(key) {
                fs::remove_file(child.path())
                    .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &child.path(), error))?;
                removed += 1;
            }
        }
        if fs::read_dir(&child_directory).is_ok_and(|mut entries| entries.next().is_none()) {
            fs::remove_dir(&child_directory)
                .map_err(|error| io_diagnostic("AMAMO_CACHE_PRUNE", &child_directory, error))?;
        }
    }
    Ok(removed)
}

pub fn remove(directory: &Path, key: &str) -> Result<(), Vec<Diagnostic>> {
    let path = entry_path(directory, key)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_diagnostic("AMAMO_CACHE_WRITE", &path, error)),
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
    Ok(directory.join(&key[..2]).join(format!("{key}.json")))
}

fn valid_key(key: &str) -> bool {
    key.len() >= 2 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn io_diagnostic(code: &str, path: &Path, error: std::io::Error) -> Vec<Diagnostic> {
    vec![Diagnostic::error(code, path.to_str(), error.to_string())]
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::read;

    #[test]
    fn corrupt_cache_is_removed_and_becomes_a_miss() {
        let directory =
            std::env::temp_dir().join(format!("amamo-mdx-cache-{}", std::process::id()));
        let entry_directory = directory.join("ab");
        let entry = entry_directory.join("abcdef.json");
        fs::create_dir_all(&entry_directory).unwrap();
        fs::write(&entry, b"{truncated").unwrap();

        let result = read(&directory, "abcdef").unwrap();

        assert!(result.value.is_none());
        assert_eq!(result.diagnostic.unwrap().code, "AMAMO_CACHE_CORRUPT");
        assert!(!entry.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
