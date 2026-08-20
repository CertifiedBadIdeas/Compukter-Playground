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
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use compukter_playground::profile::{MachineProfile, RuntimeMode};
use compukter_playground::runtime::{RuntimeCommand, RuntimeHandle, RuntimeOutcome};
use tempfile::tempdir;

const BANNER: &[u8] = b"Compukter Playground UART ready\r\n";

#[test]
fn firmware_prints_banner_sleeps_and_echoes_uart_input() {
    let elf = build_firmware();
    let profile = MachineProfile {
        initial_mode: RuntimeMode::Unbounded,
        ..MachineProfile::default()
    };
    let instruction_budget = profile.clock.instructions_per_tick;
    let runtime = RuntimeHandle::spawn(profile, elf, None).unwrap();

    let idle = runtime.wait_for(
        |snapshot| {
            snapshot.terminal.raw_bytes.as_slice() == BANNER
                && snapshot.outcome == RuntimeOutcome::WaitingForInterrupt
        },
        Duration::from_secs(2),
    );
    assert!(idle.error.is_none());
    assert!(
        idle.inspection.hart.retired_instructions < instruction_budget,
        "interrupt-driven banner transmission retired {} instructions, one quantum is {instruction_budget}",
        idle.inspection.hart.retired_instructions
    );

    let input = b"Echo me!\r\n";
    runtime
        .command(RuntimeCommand::SendUart(input.to_vec()))
        .unwrap();
    let echoed = runtime.wait_for(
        |snapshot| {
            snapshot.terminal.raw_bytes.ends_with(input)
                && snapshot.outcome == RuntimeOutcome::WaitingForInterrupt
        },
        Duration::from_secs(2),
    );

    assert_eq!(&echoed.terminal.raw_bytes[..BANNER.len()], BANNER);
    assert!(echoed.error.is_none());
}

fn build_firmware() -> Vec<u8> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temporary = tempdir().unwrap();
    let output = temporary.path().join("firmware.elf");
    let status = Command::new(repository.join("scripts/build-firmware.sh"))
        .arg(&output)
        .current_dir(repository)
        .status()
        .unwrap();
    assert!(status.success(), "firmware build failed with {status}");
    fs::read(output).unwrap()
}
