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

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEntry {
    name: String,
    path: PathBuf,
}

impl ProfileEntry {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCatalog {
    entries: Vec<ProfileEntry>,
}

impl ProfileCatalog {
    pub fn scan(directory: &Path) -> Result<Self, ProfileCatalogError> {
        let mut entries = Vec::new();
        let directory_entries =
            fs::read_dir(directory).map_err(|source| ProfileCatalogError::ReadDirectory {
                path: directory.to_owned(),
                source,
            })?;

        for entry in directory_entries {
            let entry = entry.map_err(|source| ProfileCatalogError::ReadDirectoryEntry {
                path: directory.to_owned(),
                source,
            })?;
            let file_type =
                entry
                    .file_type()
                    .map_err(|source| ProfileCatalogError::ReadDirectoryEntry {
                        path: directory.to_owned(),
                        source,
                    })?;
            let path = entry.path();
            if !file_type.is_file() || path.extension().is_none_or(|extension| extension != "toml")
            {
                continue;
            }
            entries.push(ProfileEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path,
            });
        }

        entries.sort_by(|left, right| left.name.cmp(&right.name));
        if entries.is_empty() {
            return Err(ProfileCatalogError::Empty {
                path: directory.to_owned(),
            });
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[ProfileEntry] {
        &self.entries
    }

    pub fn startup_entry(&self) -> &ProfileEntry {
        self.entries
            .iter()
            .find(|entry| entry.name == "default.toml")
            .unwrap_or(&self.entries[0])
    }

    pub fn find(&self, name: &str) -> Option<&ProfileEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

#[derive(Debug, Error)]
pub enum ProfileCatalogError {
    #[error("could not read profile directory {path}: {source}")]
    ReadDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not read an entry in profile directory {path}: {source}")]
    ReadDirectoryEntry {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("profile directory {path} contains no TOML profiles")]
    Empty { path: PathBuf },
}
