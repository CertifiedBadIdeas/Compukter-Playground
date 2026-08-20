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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::profile::{MachineProfile, ProfileError};
use crate::profile_catalog::{ProfileCatalog, ProfileEntry};
use crate::runtime::{RuntimeCommand, RuntimeError, RuntimeHandle, RuntimeSnapshot};

#[derive(Debug, Default)]
pub struct PlaygroundViewModel {
    catalog: Option<ProfileCatalog>,
    profile_path: Option<PathBuf>,
    profile: Option<MachineProfile>,
    runtime: Option<RuntimeHandle>,
    status_message: Option<String>,
}

impl PlaygroundViewModel {
    pub fn from_profiles_dir(directory: impl AsRef<Path>) -> Self {
        let mut view_model = Self::default();
        match ProfileCatalog::scan(directory.as_ref()) {
            Ok(catalog) => {
                let startup_profile = catalog.startup_entry().name().to_owned();
                view_model.catalog = Some(catalog);
                if let Err(error) = view_model.select_profile(&startup_profile) {
                    view_model.set_status_error(error);
                }
            }
            Err(error) => view_model.set_status_error(error),
        }
        view_model
    }

    pub fn select_profile(&mut self, name: &str) -> Result<(), ViewModelError> {
        let path = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.find(name))
            .map(|entry| entry.path().to_owned())
            .ok_or_else(|| ViewModelError::UnknownProfile(name.to_owned()))?;
        self.open_profile(&path)
    }

    pub fn open_profile(&mut self, path: &Path) -> Result<(), ViewModelError> {
        let profile = MachineProfile::load(path)?;
        let firmware_path = profile.resolve_firmware(path);
        let elf = std::fs::read(&firmware_path).map_err(|source| ViewModelError::FirmwareIo {
            path: firmware_path,
            source,
        })?;
        let runtime = RuntimeHandle::spawn(profile.clone(), elf, None)?;
        self.runtime = Some(runtime);
        self.profile = Some(profile);
        self.profile_path = Some(path.to_owned());
        self.status_message = None;
        Ok(())
    }

    pub fn save_profile(&mut self) -> Result<(), ViewModelError> {
        let path = self
            .profile_path
            .as_deref()
            .ok_or(ViewModelError::NoProfile)?;
        self.profile
            .as_ref()
            .ok_or(ViewModelError::NoProfile)?
            .save(path)?;
        self.status_message = Some(format!("Saved {}", path.display()));
        Ok(())
    }

    pub fn command(&mut self, command: RuntimeCommand) -> Result<(), ViewModelError> {
        self.runtime
            .as_ref()
            .ok_or(ViewModelError::NoMachine)?
            .command(command)?;
        Ok(())
    }

    pub fn snapshot(&self) -> Option<Arc<RuntimeSnapshot>> {
        self.runtime.as_ref().and_then(RuntimeHandle::snapshot)
    }

    pub fn profile_path(&self) -> Option<&Path> {
        self.profile_path.as_deref()
    }

    pub fn profiles(&self) -> &[ProfileEntry] {
        self.catalog.as_ref().map_or(&[], ProfileCatalog::entries)
    }

    pub fn profile(&self) -> Option<&MachineProfile> {
        self.profile.as_ref()
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    pub fn set_status_error(&mut self, error: impl std::fmt::Display) {
        self.status_message = Some(error.to_string());
    }
}

#[derive(Debug, Error)]
pub enum ViewModelError {
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error("could not read firmware {path}: {source}")]
    FirmwareIo {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("no machine profile is open")]
    NoProfile,
    #[error("profile {0} is not present in the profile catalog")]
    UnknownProfile(String),
    #[error("no VM is running")]
    NoMachine,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{PlaygroundViewModel, ViewModelError};
    use crate::profile::MachineProfile;
    use crate::profile_catalog::ProfileCatalog;
    use crate::runtime::RuntimeCommand;
    use tempfile::tempdir;

    #[test]
    fn commands_require_an_open_machine() {
        let mut view_model = PlaygroundViewModel::default();

        assert!(matches!(
            view_model.command(RuntimeCommand::Step),
            Err(ViewModelError::NoMachine)
        ));
        assert!(view_model.snapshot().is_none());
    }

    #[test]
    fn startup_keeps_catalog_when_default_profile_cannot_load() {
        let temporary = tempdir().unwrap();
        let profiles = temporary.path().join("profiles");
        fs::create_dir(&profiles).unwrap();
        MachineProfile::default()
            .save(&profiles.join("default.toml"))
            .unwrap();

        let view_model = PlaygroundViewModel::from_profiles_dir(&profiles);

        assert_eq!(view_model.profiles()[0].name(), "default.toml");
        assert!(view_model.profile_path().is_none());
        assert!(view_model
            .status_message()
            .unwrap()
            .contains("could not read firmware"));
    }

    #[test]
    fn failed_selection_preserves_active_profile() {
        let temporary = tempdir().unwrap();
        let profiles = temporary.path().join("profiles");
        fs::create_dir(&profiles).unwrap();
        let active_path = profiles.join("active.toml");
        let broken_path = profiles.join("broken.toml");
        let profile = MachineProfile::default();
        profile.save(&active_path).unwrap();
        profile.save(&broken_path).unwrap();

        let mut view_model = PlaygroundViewModel {
            catalog: Some(ProfileCatalog::scan(&profiles).unwrap()),
            profile_path: Some(active_path.clone()),
            profile: Some(profile.clone()),
            runtime: None,
            status_message: None,
        };

        assert!(view_model.select_profile("broken.toml").is_err());
        assert_eq!(view_model.profile_path(), Some(active_path.as_path()));
        assert_eq!(view_model.profile(), Some(&profile));
    }
}
