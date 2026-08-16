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
use std::io::Write;
use std::path::{Path, PathBuf};

use compukter_vm::rv32_machine::{
    Rv32DbtCodeAlignment, Rv32DbtRegisterProfile, Rv32ExecutionBackendConfig, CONTROL_BASE,
    DEBUG_BASE, PLIC_BASE, TIMER_BASE,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROFILE_SCHEMA_VERSION: u32 = 1;
const UART_REGISTER_BYTES: u32 = 8;
const PLATFORM_MMIO_BYTES: u32 = 256;
const PLIC_BYTES: u32 = 0x0020_1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineProfile {
    pub schema_version: u32,
    pub firmware: FirmwareProfile,
    pub machine: MachineConfigProfile,
    pub clock: ClockProfile,
    pub uart: UartProfile,
    pub initial_mode: RuntimeMode,
}

impl Default for MachineProfile {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            firmware: FirmwareProfile {
                elf: PathBuf::from("firmware.elf"),
            },
            machine: MachineConfigProfile {
                ram_bytes: 16 * 1024 * 1024,
                debug_limit: 4096,
                backend: BackendProfile::default(),
            },
            clock: ClockProfile {
                instructions_per_tick: 100_000,
                timer_units_per_tick: 1,
            },
            uart: UartProfile {
                base: 0x1000_1000,
                connected: true,
            },
            initial_mode: RuntimeMode::Realtime,
        }
    }
}

impl MachineProfile {
    pub fn from_toml(text: &str) -> Result<Self, ProfileError> {
        let profile: Self = toml::from_str(text)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn to_toml(&self) -> Result<String, ProfileError> {
        self.validate()?;
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        Self::from_toml(&fs::read_to_string(path)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ProfileError> {
        let contents = self.to_toml()?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ProfileError::InvalidProfilePath(path.to_owned()))?;
        let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(ProfileError::Io)
    }

    pub fn resolve_firmware(&self, profile_path: &Path) -> PathBuf {
        profile_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&self.firmware.elf)
    }

    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(ProfileError::UnsupportedSchema(self.schema_version));
        }
        if self.firmware.elf.is_absolute() {
            return Err(ProfileError::AbsoluteFirmwarePath(
                self.firmware.elf.clone(),
            ));
        }
        if self.machine.ram_bytes == 0 || self.machine.ram_bytes > CONTROL_BASE as usize {
            return Err(ProfileError::InvalidRamSize(self.machine.ram_bytes));
        }
        if self.clock.instructions_per_tick == 0 {
            return Err(ProfileError::ZeroBudget("instructions_per_tick"));
        }
        if self.clock.timer_units_per_tick == 0 {
            return Err(ProfileError::ZeroBudget("timer_units_per_tick"));
        }
        self.machine.backend.validate()?;

        let Some(uart_end) = self.uart.base.checked_add(UART_REGISTER_BYTES) else {
            return Err(ProfileError::InvalidUartBase {
                base: self.uart.base,
                reason: "register range overflows the RV32 address space",
            });
        };
        let in_ram = (self.uart.base as usize) < self.machine.ram_bytes;
        let overlaps_platform = [CONTROL_BASE, DEBUG_BASE, TIMER_BASE]
            .into_iter()
            .any(|base| ranges_overlap(self.uart.base, uart_end, base, base + PLATFORM_MMIO_BYTES));
        let overlaps_plic =
            ranges_overlap(self.uart.base, uart_end, PLIC_BASE, PLIC_BASE + PLIC_BYTES);
        if in_ram || overlaps_platform || overlaps_plic {
            return Err(ProfileError::InvalidUartBase {
                base: self.uart.base,
                reason: "register range overlaps RAM or built-in platform MMIO",
            });
        }
        Ok(())
    }
}

const fn ranges_overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    a_start < b_end && b_start < a_end
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmwareProfile {
    pub elf: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineConfigProfile {
    pub ram_bytes: usize,
    pub debug_limit: usize,
    pub backend: BackendProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BackendProfile {
    Cached {
        sets: usize,
    },
    Predecoded,
    BlockCached {
        sets: usize,
        max_instructions: usize,
    },
    DirectDbt {
        max_instructions: usize,
        scratch_bytes: usize,
    },
    CachedDbt {
        sets: usize,
        max_instructions: usize,
        scratch_bytes: usize,
        cache_bytes: usize,
        alignment: usize,
    },
}

impl Default for BackendProfile {
    fn default() -> Self {
        Self::CachedDbt {
            sets: 256,
            max_instructions: 16,
            scratch_bytes: 8 * 1024,
            cache_bytes: 128 * 1024,
            alignment: 64,
        }
    }
}

impl BackendProfile {
    pub fn execution_config(&self) -> Rv32ExecutionBackendConfig {
        match *self {
            Self::Cached { sets } => Rv32ExecutionBackendConfig::Cached { sets },
            Self::Predecoded => Rv32ExecutionBackendConfig::Predecoded,
            Self::BlockCached {
                sets,
                max_instructions,
            } => Rv32ExecutionBackendConfig::BlockCached {
                sets,
                max_instructions,
            },
            Self::DirectDbt {
                max_instructions,
                scratch_bytes,
            } => Rv32ExecutionBackendConfig::DirectDbt {
                max_instructions,
                scratch_bytes,
            },
            Self::CachedDbt {
                sets,
                max_instructions,
                scratch_bytes,
                cache_bytes,
                alignment,
            } => Rv32ExecutionBackendConfig::CachedDbt {
                sets,
                max_instructions,
                scratch_bytes,
                cache_bytes,
                code_alignment: Rv32DbtCodeAlignment::BlockBase(alignment),
                register_profile: Rv32DbtRegisterProfile::RcxOverflow8,
            },
        }
    }

    fn validate(&self) -> Result<(), ProfileError> {
        let (sets, max_instructions) = match self {
            Self::Cached { sets } => (Some(*sets), None),
            Self::Predecoded => (None, None),
            Self::BlockCached {
                sets,
                max_instructions,
            } => (Some(*sets), Some(*max_instructions)),
            Self::DirectDbt {
                max_instructions, ..
            } => (None, Some(*max_instructions)),
            Self::CachedDbt {
                sets,
                max_instructions,
                scratch_bytes,
                cache_bytes,
                alignment,
            } => {
                for (name, value) in [
                    ("scratch_bytes", *scratch_bytes),
                    ("cache_bytes", *cache_bytes),
                    ("alignment", *alignment),
                ] {
                    if value == 0 {
                        return Err(ProfileError::ZeroBackendParameter(name));
                    }
                }
                if !alignment.is_power_of_two() {
                    return Err(ProfileError::NonPowerOfTwo("alignment", *alignment));
                }
                (Some(*sets), Some(*max_instructions))
            }
        };
        if let Some(sets) = sets {
            if sets == 0 {
                return Err(ProfileError::ZeroBackendParameter("sets"));
            }
            if !sets.is_power_of_two() {
                return Err(ProfileError::NonPowerOfTwo("sets", sets));
            }
        }
        if max_instructions == Some(0) {
            return Err(ProfileError::ZeroBackendParameter("max_instructions"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockProfile {
    pub instructions_per_tick: u64,
    pub timer_units_per_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UartProfile {
    pub base: u32,
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    Realtime,
    Unbounded,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML profile: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("could not encode TOML profile: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("unsupported profile schema version {0}")]
    UnsupportedSchema(u32),
    #[error("firmware path must be relative to the profile: {0}")]
    AbsoluteFirmwarePath(PathBuf),
    #[error("invalid RAM size {0}")]
    InvalidRamSize(usize),
    #[error("{0} must be greater than zero")]
    ZeroBudget(&'static str),
    #[error("backend parameter {0} must be greater than zero")]
    ZeroBackendParameter(&'static str),
    #[error("backend parameter {0} must be a power of two, got {1}")]
    NonPowerOfTwo(&'static str, usize),
    #[error("invalid UART base {base:#010x}: {reason}")]
    InvalidUartBase { base: u32, reason: &'static str },
    #[error("profile path has no usable file name: {0}")]
    InvalidProfilePath(PathBuf),
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{MachineProfile, ProfileError, PROFILE_SCHEMA_VERSION};

    #[test]
    fn profile_round_trips_and_resolves_firmware_relative_to_profile() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("machine.toml");
        let profile = MachineProfile::default();

        profile.save(&path).unwrap();
        let loaded = MachineProfile::load(&path).unwrap();

        assert_eq!(loaded, profile);
        assert_eq!(
            loaded.resolve_firmware(&path),
            directory.path().join("firmware.elf")
        );
        assert!(!directory.path().join("machine.toml.tmp").exists());
    }

    #[test]
    fn rejects_unknown_schema_before_using_profile() {
        let text = MachineProfile::default().to_toml().unwrap().replace(
            &format!("schema_version = {PROFILE_SCHEMA_VERSION}"),
            "schema_version = 999",
        );

        assert!(matches!(
            MachineProfile::from_toml(&text),
            Err(ProfileError::UnsupportedSchema(999))
        ));
    }

    #[test]
    fn rejects_uart_inside_ram_or_platform_mmio() {
        let mut in_ram = MachineProfile::default();
        in_ram.uart.base = 0x1000;
        assert!(matches!(
            in_ram.validate(),
            Err(ProfileError::InvalidUartBase { .. })
        ));

        let mut in_platform = MachineProfile::default();
        in_platform.uart.base = compukter_vm::rv32_machine::TIMER_BASE;
        assert!(matches!(
            in_platform.validate(),
            Err(ProfileError::InvalidUartBase { .. })
        ));
    }

    #[test]
    fn rejects_zero_runtime_budgets_and_absolute_firmware_paths() {
        let mut profile = MachineProfile::default();
        profile.clock.instructions_per_tick = 0;
        assert!(matches!(
            profile.validate(),
            Err(ProfileError::ZeroBudget(_))
        ));

        let mut profile = MachineProfile::default();
        profile.firmware.elf = Path::new("/tmp/firmware.elf").to_owned();
        assert!(matches!(
            profile.validate(),
            Err(ProfileError::AbsoluteFirmwarePath(_))
        ));
    }
}
