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
use std::thread;
use std::time::{Duration, Instant};

use compukter_playground::profile::{MachineProfile, RuntimeMode};
use compukter_playground::runtime::{RuntimeCommand, RuntimeHandle, RuntimeOutcome};
use tempfile::tempdir;

#[test]
#[ignore = "builds pinned upstream NuttX before booting it"]
fn nuttx_boots_nsh_and_runs_builtin_over_uart() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temporary = tempdir().unwrap();
    let output = temporary.path().join("nuttx.elf");
    let status = Command::new(repository.join("scripts/build-nuttx.sh"))
        .arg(&output)
        .current_dir(repository)
        .status()
        .unwrap();
    assert!(status.success(), "NuttX build failed with {status}");

    let mut profile = MachineProfile::load(&repository.join("profiles/nuttx.toml")).unwrap();
    profile.initial_mode = RuntimeMode::Unbounded;
    let runtime = RuntimeHandle::spawn(profile, fs::read(output).unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);

    let prompt = loop {
        if let Some(snapshot) = runtime.snapshot() {
            if snapshot
                .terminal
                .raw_bytes
                .windows(4)
                .any(|part| part == b"nsh>")
            {
                break snapshot;
            }
            if snapshot.error.is_some()
                || matches!(
                    snapshot.outcome,
                    RuntimeOutcome::Faulted
                        | RuntimeOutcome::Halted(_)
                        | RuntimeOutcome::Panicked(_)
                )
            {
                panic!("NuttX stopped before NSH:\n{}", diagnostic(&snapshot));
            }
            if Instant::now() >= deadline {
                panic!("NuttX did not reach NSH:\n{}", diagnostic(&snapshot));
            }
        }
        thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(prompt.outcome, RuntimeOutcome::WaitingForInterrupt);
    assert!(prompt.inspection.hart.waiting_for_interrupt);
    assert_eq!(prompt.inspection.hart.csrs.mie & 0x880, 0x880);
    assert!(prompt.inspection.timer.compare > prompt.inspection.timer.time);
    assert_eq!(prompt.inspection.plic.source_count, 1);
    assert_eq!(prompt.inspection.plic.sources[0].priority, 1);
    assert!(prompt.inspection.plic.sources[0].enabled);

    let initial_time = prompt.inspection.timer.time;
    let initial_compare = prompt.inspection.timer.compare;
    let rearmed = wait_for_snapshot(&runtime, Duration::from_secs(10), |snapshot| {
        snapshot.inspection.timer.time > initial_time
            && snapshot.inspection.timer.time >= initial_compare
            && snapshot.inspection.timer.compare > initial_compare
            && snapshot.inspection.timer.compare > snapshot.inspection.timer.time
            && snapshot.outcome == RuntimeOutcome::WaitingForInterrupt
            && snapshot.inspection.hart.waiting_for_interrupt
    });
    assert!(rearmed.error.is_none());

    runtime
        .command(RuntimeCommand::SendUart(b"hello\n".to_vec()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(snapshot) = runtime.snapshot() {
            if snapshot
                .terminal
                .raw_bytes
                .windows(b"Hello from NuttX on Compukter-VM".len())
                .any(|part| part == b"Hello from NuttX on Compukter-VM")
            {
                break;
            }
            if snapshot.error.is_some() || Instant::now() >= deadline {
                panic!("NuttX did not execute hello:\n{}", diagnostic(&snapshot));
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    let prompts_before_help = occurrence_count(
        &runtime
            .snapshot()
            .expect("runtime snapshot after hello")
            .terminal
            .raw_bytes,
        b"nsh>",
    );
    runtime
        .command(RuntimeCommand::SendUart(b"help\n".to_vec()))
        .unwrap();
    let help = wait_for_snapshot(&runtime, Duration::from_secs(10), |snapshot| {
        snapshot
            .terminal
            .raw_bytes
            .windows(b"Builtin Apps:".len())
            .any(|part| part == b"Builtin Apps:")
            && occurrence_count(&snapshot.terminal.raw_bytes, b"nsh>") > prompts_before_help
            && snapshot.outcome == RuntimeOutcome::WaitingForInterrupt
            && snapshot.inspection.hart.waiting_for_interrupt
    });
    assert!(help.error.is_none());
    assert_eq!(help.inspection.plic.sources[0].priority, 1);
    assert!(help.inspection.plic.sources[0].enabled);
    assert!(!help.inspection.plic.sources[0].in_flight);
}

fn wait_for_snapshot(
    runtime: &RuntimeHandle,
    timeout: Duration,
    predicate: impl Fn(&compukter_playground::runtime::RuntimeSnapshot) -> bool,
) -> std::sync::Arc<compukter_playground::runtime::RuntimeSnapshot> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(snapshot) = runtime.snapshot() {
            if predicate(&snapshot) {
                return snapshot;
            }
            if snapshot.error.is_some()
                || matches!(
                    snapshot.outcome,
                    RuntimeOutcome::Faulted
                        | RuntimeOutcome::Halted(_)
                        | RuntimeOutcome::Panicked(_)
                )
                || Instant::now() >= deadline
            {
                panic!(
                    "NuttX did not reach expected state:\n{}",
                    diagnostic(&snapshot)
                );
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn occurrence_count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|part| *part == needle)
        .count()
}

fn diagnostic(snapshot: &compukter_playground::runtime::RuntimeSnapshot) -> String {
    let terminal = &snapshot.terminal.raw_bytes;
    let tail = &terminal[terminal.len().saturating_sub(4096)..];
    format!(
        "outcome={:?} error={:?}\nhart={:#?}\ntimer={:#?}\nuart={:#?}\nterminal tail:\n{}",
        snapshot.outcome,
        snapshot.error,
        snapshot.inspection.hart,
        snapshot.inspection.timer,
        snapshot.uart,
        String::from_utf8_lossy(tail)
    )
}
