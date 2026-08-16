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
use crate::runtime::{RuntimeCommand, RuntimeError, RuntimeHandle, RuntimeSnapshot};

#[derive(Debug, Default)]
pub struct PlaygroundViewModel {
    profile_path: Option<PathBuf>,
    profile: Option<MachineProfile>,
    runtime: Option<RuntimeHandle>,
    status_message: Option<String>,
}

impl PlaygroundViewModel {
    pub fn open_profile(&mut self, path: &Path) -> Result<(), ViewModelError> {
        let profile = MachineProfile::load(path)?;
        let firmware_path = profile.resolve_firmware(path);
        let elf = std::fs::read(&firmware_path).map_err(|source| ViewModelError::FirmwareIo {
            path: firmware_path,
            source,
        })?;
        let runtime = RuntimeHandle::spawn(profile.clone(), elf)?;
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
    #[error("no VM is running")]
    NoMachine,
}

#[cfg(test)]
mod tests {
    use super::{PlaygroundViewModel, ViewModelError};
    use crate::runtime::RuntimeCommand;

    #[test]
    fn commands_require_an_open_machine() {
        let mut view_model = PlaygroundViewModel::default();

        assert!(matches!(
            view_model.command(RuntimeCommand::Step),
            Err(ViewModelError::NoMachine)
        ));
        assert!(view_model.snapshot().is_none());
    }
}
