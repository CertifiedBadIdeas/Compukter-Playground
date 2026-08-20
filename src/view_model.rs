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

use crate::disk_image::{DiskImageError, LoadedDiskImage};
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
        let mut prepared = PreparedLaunch::load(path)?;
        if self
            .profile
            .as_ref()
            .and_then(|profile| profile.disk.as_ref())
            .is_some_and(|disk| !disk.read_only)
        {
            self.runtime
                .as_ref()
                .ok_or(ViewModelError::NoMachine)?
                .save_disk()?;
            prepared.reload_disk()?;
        }
        let runtime = RuntimeHandle::spawn(prepared.profile.clone(), prepared.elf, prepared.disk)?;
        let previous = self.runtime.replace(runtime);
        self.profile = Some(prepared.profile);
        self.profile_path = Some(prepared.path);
        self.status_message = None;
        if let Some(previous) = previous {
            previous.stop_without_save();
        }
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

    pub fn can_save_disk(&self) -> bool {
        self.profile
            .as_ref()
            .and_then(|profile| profile.disk.as_ref())
            .is_some_and(|disk| !disk.read_only)
    }

    pub fn save_disk(&mut self) -> Result<(), ViewModelError> {
        let path = self
            .runtime
            .as_ref()
            .ok_or(ViewModelError::NoMachine)?
            .save_disk()?;
        self.status_message = Some(format!("Saved disk {}", path.display()));
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

struct PreparedLaunch {
    path: PathBuf,
    profile: MachineProfile,
    elf: Vec<u8>,
    disk: Option<LoadedDiskImage>,
}

impl PreparedLaunch {
    fn load(path: &Path) -> Result<Self, ViewModelError> {
        let profile = MachineProfile::load(path)?;
        let firmware_path = profile.resolve_firmware(path);
        let elf = std::fs::read(&firmware_path).map_err(|source| ViewModelError::FirmwareIo {
            path: firmware_path,
            source,
        })?;
        let disk = profile
            .disk
            .as_ref()
            .map(|disk| LoadedDiskImage::load(path, disk))
            .transpose()?;
        Ok(Self {
            path: path.to_owned(),
            profile,
            elf,
            disk,
        })
    }

    fn reload_disk(&mut self) -> Result<(), DiskImageError> {
        self.disk = self
            .profile
            .disk
            .as_ref()
            .map(|disk| LoadedDiskImage::load(&self.path, disk))
            .transpose()?;
        Ok(())
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
    Disk(#[from] DiskImageError),
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
    use std::path::{Path, PathBuf};

    use super::{PlaygroundViewModel, ViewModelError};
    use crate::profile::{BackendProfile, DiskProfile, MachineProfile};
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

    #[test]
    fn opens_profile_with_relative_disk_and_saves_it() {
        let temporary = tempdir().unwrap();
        let path =
            write_runnable_profile(temporary.path(), "active.toml", Some(("disk.img", false)));
        let mut view_model = PlaygroundViewModel::default();

        view_model.open_profile(&path).unwrap();

        assert!(view_model.can_save_disk());
        view_model.save_disk().unwrap();
        assert!(view_model.status_message().unwrap().contains("Saved disk"));
    }

    #[test]
    fn failed_pre_switch_save_preserves_the_active_profile() {
        let temporary = tempdir().unwrap();
        let active =
            write_runnable_profile(temporary.path(), "active.toml", Some(("active.img", false)));
        let next = write_runnable_profile(temporary.path(), "next.toml", None);
        let mut view_model = PlaygroundViewModel::default();
        view_model.open_profile(&active).unwrap();
        let image = temporary.path().join("active.img");
        fs::remove_file(&image).unwrap();
        fs::create_dir(&image).unwrap();

        assert!(view_model.open_profile(&next).is_err());
        assert_eq!(view_model.profile_path(), Some(active.as_path()));
        assert!(view_model.snapshot().unwrap().paused);
    }

    #[test]
    fn invalid_new_disk_does_not_replace_the_active_profile() {
        let temporary = tempdir().unwrap();
        let active = write_runnable_profile(temporary.path(), "active.toml", None);
        let invalid = write_runnable_profile(
            temporary.path(),
            "invalid.toml",
            Some(("invalid.img", false)),
        );
        fs::write(temporary.path().join("invalid.img"), vec![0; 511]).unwrap();
        let mut view_model = PlaygroundViewModel::default();
        view_model.open_profile(&active).unwrap();

        assert!(view_model.open_profile(&invalid).is_err());
        assert_eq!(view_model.profile_path(), Some(active.as_path()));
        view_model.command(RuntimeCommand::SetPaused(true)).unwrap();
    }

    #[test]
    fn reopening_a_shared_image_does_not_restore_bytes_loaded_before_save() {
        let temporary = tempdir().unwrap();
        let profile =
            write_runnable_profile(temporary.path(), "active.toml", Some(("shared.img", false)));
        let image = temporary.path().join("shared.img");
        let mut view_model = PlaygroundViewModel::default();
        view_model.open_profile(&profile).unwrap();
        view_model
            .runtime
            .as_ref()
            .unwrap()
            .mutate_disk_for_test(0, 0x66)
            .unwrap();

        view_model.open_profile(&profile).unwrap();
        view_model.save_disk().unwrap();

        assert_eq!(fs::read(image).unwrap()[0], 0x66);
    }

    fn write_runnable_profile(directory: &Path, name: &str, disk: Option<(&str, bool)>) -> PathBuf {
        let firmware_name = format!("{name}.elf");
        fs::write(directory.join(&firmware_name), runnable_elf()).unwrap();
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        profile.firmware.elf = PathBuf::from(firmware_name);
        profile.disk = disk.map(|(image, read_only)| {
            let image_path = directory.join(image);
            if !image_path.exists() {
                fs::write(&image_path, vec![0x11; 512]).unwrap();
            }
            DiskProfile {
                image: PathBuf::from(image),
                read_only,
            }
        });
        let path = directory.join(name);
        profile.save(&path).unwrap();
        path
    }

    fn runnable_elf() -> Vec<u8> {
        const ELF_HEADER: usize = 52;
        const PROGRAM_HEADER: usize = 32;
        const PAGE: usize = 4096;
        let code = 0x0000_006f_u32.to_le_bytes();
        let mut elf = vec![0; PAGE + code.len()];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 1;
        elf[5] = 1;
        elf[6] = 1;
        put_u16(&mut elf, 16, 2);
        put_u16(&mut elf, 18, 243);
        put_u32(&mut elf, 20, 1);
        put_u32(&mut elf, 24, 0x1000);
        put_u32(&mut elf, 28, ELF_HEADER as u32);
        put_u16(&mut elf, 40, ELF_HEADER as u16);
        put_u16(&mut elf, 42, PROGRAM_HEADER as u16);
        put_u16(&mut elf, 44, 1);
        put_u32(&mut elf, ELF_HEADER, 1);
        put_u32(&mut elf, ELF_HEADER + 4, PAGE as u32);
        put_u32(&mut elf, ELF_HEADER + 8, 0x1000);
        put_u32(&mut elf, ELF_HEADER + 12, 0x1000);
        put_u32(&mut elf, ELF_HEADER + 16, code.len() as u32);
        put_u32(&mut elf, ELF_HEADER + 20, code.len() as u32);
        put_u32(&mut elf, ELF_HEADER + 24, 0b101);
        put_u32(&mut elf, ELF_HEADER + 28, PAGE as u32);
        elf[PAGE..].copy_from_slice(&code);
        elf
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
