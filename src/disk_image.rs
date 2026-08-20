/*
 * The Compukter Kraft Developers
 *
 * Copyright (C) 2026 Vsevolod Petrov (lazyhat)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 */

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use compukter_vm_devices::virtio::VIRTIO_BLOCK_SECTOR_SIZE;
use thiserror::Error;

use crate::profile::DiskProfile;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct LoadedDiskImage {
    path: PathBuf,
    bytes: Vec<u8>,
    read_only: bool,
}

impl LoadedDiskImage {
    pub fn load(profile_path: &Path, profile: &DiskProfile) -> Result<Self, DiskImageError> {
        let path = profile_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&profile.image);
        let bytes = fs::read(&path).map_err(|source| DiskImageError::Io {
            operation: "read",
            path: path.clone(),
            source,
        })?;
        validate_capacity(&path, &bytes)?;
        Ok(Self {
            path,
            bytes,
            read_only: profile.read_only,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn into_parts(self) -> (PathBuf, Vec<u8>, bool) {
        (self.path, self.bytes, self.read_only)
    }

}

pub fn persist_atomic(path: &Path, bytes: &[u8]) -> Result<(), DiskImageError> {
    validate_capacity(path, bytes)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DiskImageError::InvalidPath(path.to_owned()))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| io_error("create temporary", &temporary, source))?;
        file.write_all(bytes)
            .map_err(|source| io_error("write", &temporary, source))?;
        file.sync_all()
            .map_err(|source| io_error("synchronize", &temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| io_error("replace", path, source))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_capacity(path: &Path, bytes: &[u8]) -> Result<(), DiskImageError> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(VIRTIO_BLOCK_SECTOR_SIZE) {
        return Err(DiskImageError::InvalidCapacity {
            path: path.to_owned(),
            bytes: bytes.len(),
        });
    }
    Ok(())
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> DiskImageError {
    DiskImageError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug, Error)]
pub enum DiskImageError {
    #[error("could not {operation} disk image {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "disk image {path} has invalid capacity {bytes}; expected a positive multiple of 512 bytes"
    )]
    InvalidCapacity { path: PathBuf, bytes: usize },
    #[error("disk image {0} is read-only")]
    ReadOnly(PathBuf),
    #[error("disk image path has no usable file name: {0}")]
    InvalidPath(PathBuf),
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use compukter_vm_devices::virtio::VIRTIO_BLOCK_SECTOR_SIZE;
    use tempfile::tempdir;

    use super::{persist_atomic, DiskImageError, LoadedDiskImage};
    use crate::profile::DiskProfile;

    #[test]
    fn loads_a_profile_relative_sector_aligned_image() {
        let temporary = tempdir().unwrap();
        let profile_path = temporary.path().join("machine.toml");
        let image_path = temporary.path().join("disk.img");
        fs::write(&image_path, vec![0x5a; VIRTIO_BLOCK_SECTOR_SIZE]).unwrap();

        let loaded = LoadedDiskImage::load(
            &profile_path,
            &DiskProfile {
                image: PathBuf::from("disk.img"),
                read_only: false,
            },
        )
        .unwrap();

        assert_eq!(loaded.path(), image_path);
        assert_eq!(loaded.bytes(), vec![0x5a; VIRTIO_BLOCK_SECTOR_SIZE]);
        assert!(!loaded.read_only());
    }

    #[test]
    fn rejects_empty_and_unaligned_images() {
        let temporary = tempdir().unwrap();
        let profile_path = temporary.path().join("machine.toml");
        for (name, bytes) in [("empty.img", vec![]), ("short.img", vec![0; 511])] {
            fs::write(temporary.path().join(name), bytes).unwrap();
            let error = LoadedDiskImage::load(
                &profile_path,
                &DiskProfile {
                    image: PathBuf::from(name),
                    read_only: false,
                },
            )
            .unwrap_err();
            assert!(matches!(error, DiskImageError::InvalidCapacity { .. }));
        }
    }

    #[test]
    fn missing_image_reports_the_resolved_path() {
        let temporary = tempdir().unwrap();
        let profile_path = temporary.path().join("machine.toml");
        let expected = temporary.path().join("missing.img");

        let error = LoadedDiskImage::load(
            &profile_path,
            &DiskProfile {
                image: PathBuf::from("missing.img"),
                read_only: false,
            },
        )
        .unwrap_err();

        assert!(matches!(error, DiskImageError::Io { path, .. } if path == expected));
    }

    #[test]
    fn atomic_save_replaces_the_complete_image() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("disk.img");
        fs::write(&path, vec![0x11; 512]).unwrap();

        persist_atomic(&path, &vec![0x22; 1024]).unwrap();

        assert_eq!(fs::read(&path).unwrap(), vec![0x22; 1024]);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_removes_the_temporary_file() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("disk.img");
        fs::create_dir(&path).unwrap();

        assert!(persist_atomic(&path, &vec![0x22; 512]).is_err());
        let entries: Vec<_> = fs::read_dir(temporary.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![OsString::from("disk.img")]);
        assert!(path.is_dir());
    }
}
