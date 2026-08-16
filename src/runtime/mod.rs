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

use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use compukter_vm::rv32_machine::{
    Rv32DeviceHandle, Rv32Machine, Rv32MachineBuilder, Rv32MachineConfig, Rv32MachineInspection,
    Rv32MachineOutcome,
};
use compukter_vm_devices::{Uart16550, Uart16550Diagnostics};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError};
use thiserror::Error;

use crate::profile::{MachineProfile, RuntimeMode};
use crate::terminal::{TerminalMode, TerminalProjection, TerminalSnapshot};

const REALTIME_TICK: Duration = Duration::from_millis(50);
const COMMAND_CAPACITY: usize = 64;
const UART_DRAIN_BYTES: usize = 256;

#[derive(Debug, Clone)]
pub enum RuntimeCommand {
    SetPaused(bool),
    Step,
    Reset,
    SetMode(RuntimeMode),
    SetTerminalMode(TerminalMode),
    SendUart(Vec<u8>),
    SetUartConnected(bool),
    ClearTerminal,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOutcome {
    Running,
    BudgetExhausted,
    WaitingForInterrupt,
    Halted(i32),
    Panicked(i32),
    Faulted,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub revision: u64,
    pub paused: bool,
    pub mode: RuntimeMode,
    pub outcome: RuntimeOutcome,
    pub inspection: Rv32MachineInspection,
    pub uart: Uart16550Diagnostics,
    pub terminal: TerminalSnapshot,
    pub error: Option<String>,
}

#[derive(Debug)]
struct LatestSnapshot {
    value: Mutex<Option<Arc<RuntimeSnapshot>>>,
    changed: Condvar,
}

impl LatestSnapshot {
    fn new() -> Self {
        Self {
            value: Mutex::new(None),
            changed: Condvar::new(),
        }
    }

    fn publish(&self, snapshot: RuntimeSnapshot) {
        *self.value.lock().expect("runtime snapshot mutex poisoned") = Some(Arc::new(snapshot));
        self.changed.notify_all();
    }

    fn current(&self) -> Option<Arc<RuntimeSnapshot>> {
        self.value
            .lock()
            .expect("runtime snapshot mutex poisoned")
            .clone()
    }
}

#[derive(Debug)]
pub struct RuntimeHandle {
    commands: Sender<RuntimeCommand>,
    latest: Arc<LatestSnapshot>,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeHandle {
    pub fn spawn(profile: MachineProfile, elf: Vec<u8>) -> Result<Self, RuntimeError> {
        profile.validate()?;
        let instance = MachineInstance::build(&profile, &elf)?;
        let (commands, receiver) = bounded(COMMAND_CAPACITY);
        let latest = Arc::new(LatestSnapshot::new());
        let worker_latest = Arc::clone(&latest);
        let worker = thread::Builder::new()
            .name("compukter-vm".to_string())
            .spawn(move || worker_main(profile, elf, instance, receiver, worker_latest))
            .map_err(RuntimeError::WorkerSpawn)?;
        Ok(Self {
            commands,
            latest,
            worker: Some(worker),
        })
    }

    pub fn command(&self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        self.commands
            .send(command)
            .map_err(|_| RuntimeError::WorkerStopped)
    }

    pub fn snapshot(&self) -> Option<Arc<RuntimeSnapshot>> {
        self.latest.current()
    }

    pub fn wait_for(
        &self,
        predicate: impl Fn(&RuntimeSnapshot) -> bool,
        timeout: Duration,
    ) -> Arc<RuntimeSnapshot> {
        let deadline = Instant::now() + timeout;
        let mut guard = self
            .latest
            .value
            .lock()
            .expect("runtime snapshot mutex poisoned");
        loop {
            if let Some(snapshot) = guard.as_ref() {
                if predicate(snapshot) {
                    return Arc::clone(snapshot);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for runtime state");
            let (next, timeout_result) = self
                .latest
                .changed
                .wait_timeout(guard, remaining)
                .expect("runtime snapshot mutex poisoned");
            guard = next;
            assert!(
                !timeout_result.timed_out(),
                "timed out waiting for runtime state"
            );
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(RuntimeCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct MachineInstance {
    machine: Rv32Machine,
    uart: Rv32DeviceHandle<Uart16550>,
}

impl MachineInstance {
    fn build(profile: &MachineProfile, elf: &[u8]) -> Result<Self, RuntimeError> {
        let config = Rv32MachineConfig {
            ram_size: profile.machine.ram_bytes,
            debug_limit: profile.machine.debug_limit,
            execution: profile.machine.backend.execution_config(),
        };
        let mut uart = Uart16550::new();
        if profile.uart.connected {
            uart.connect();
        }
        let mut builder = Rv32MachineBuilder::from_elf(elf, config)?;
        let (uart, _) = builder.add_mmio_device_with_irq(profile.uart.base, uart);
        let machine = builder.build()?;
        Ok(Self { machine, uart })
    }

    fn uart_mut(&mut self) -> &mut Uart16550 {
        self.machine
            .device_mut(self.uart)
            .expect("runtime UART handle invariant")
    }

    fn uart(&self) -> &Uart16550 {
        self.machine
            .device(self.uart)
            .expect("runtime UART handle invariant")
    }
}

struct WorkerState {
    profile: MachineProfile,
    elf: Vec<u8>,
    instance: MachineInstance,
    terminal: TerminalProjection,
    paused: bool,
    outcome: RuntimeOutcome,
    error: Option<String>,
    revision: u64,
}

fn worker_main(
    profile: MachineProfile,
    elf: Vec<u8>,
    instance: MachineInstance,
    commands: Receiver<RuntimeCommand>,
    latest: Arc<LatestSnapshot>,
) {
    let mut state = WorkerState {
        paused: false,
        profile,
        elf,
        instance,
        terminal: TerminalProjection::default(),
        outcome: RuntimeOutcome::Running,
        error: None,
        revision: 0,
    };
    publish(&mut state, &latest);
    let mut next_tick = Instant::now() + REALTIME_TICK;

    loop {
        let command = if state.paused {
            commands.recv().ok()
        } else {
            match state.profile.initial_mode {
                RuntimeMode::Realtime => {
                    let wait = next_tick.saturating_duration_since(Instant::now());
                    match commands.recv_timeout(wait) {
                        Ok(command) => Some(command),
                        Err(RecvTimeoutError::Timeout) => {
                            run_quantum(&mut state, true);
                            publish(&mut state, &latest);
                            next_tick += REALTIME_TICK;
                            if next_tick < Instant::now() {
                                next_tick = Instant::now() + REALTIME_TICK;
                            }
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => None,
                    }
                }
                RuntimeMode::Unbounded => match commands.try_recv() {
                    Ok(command) => Some(command),
                    Err(TryRecvError::Empty) => {
                        run_quantum(&mut state, true);
                        publish(&mut state, &latest);
                        continue;
                    }
                    Err(TryRecvError::Disconnected) => None,
                },
            }
        };

        let Some(command) = command else {
            break;
        };
        if handle_command(command, &mut state) {
            break;
        }
        publish(&mut state, &latest);
        next_tick = Instant::now() + REALTIME_TICK;
    }
}

fn handle_command(command: RuntimeCommand, state: &mut WorkerState) -> bool {
    match command {
        RuntimeCommand::SetPaused(paused) => state.paused = paused,
        RuntimeCommand::Step => {
            if state.paused {
                run_quantum_with_budget(state, 1, false);
            }
        }
        RuntimeCommand::Reset => match MachineInstance::build(&state.profile, &state.elf) {
            Ok(instance) => {
                state.instance = instance;
                state.terminal.clear();
                state.outcome = RuntimeOutcome::Running;
                state.error = None;
            }
            Err(error) => fault(state, error.to_string()),
        },
        RuntimeCommand::SetMode(mode) => state.profile.initial_mode = mode,
        RuntimeCommand::SetTerminalMode(mode) => state.terminal.set_mode(mode),
        RuntimeCommand::SendUart(bytes) => {
            state.instance.uart_mut().inject_rx(&bytes);
        }
        RuntimeCommand::SetUartConnected(connected) => {
            state.profile.uart.connected = connected;
            if connected {
                state.instance.uart_mut().connect();
            } else {
                state.instance.uart_mut().disconnect();
            }
        }
        RuntimeCommand::ClearTerminal => state.terminal.clear(),
        RuntimeCommand::Shutdown => return true,
    }
    false
}

fn run_quantum(state: &mut WorkerState, advance_time: bool) {
    run_quantum_with_budget(
        state,
        state.profile.clock.instructions_per_tick,
        advance_time,
    );
}

fn run_quantum_with_budget(state: &mut WorkerState, budget: u64, advance_time: bool) {
    if advance_time {
        state
            .instance
            .machine
            .advance_time(state.profile.clock.timer_units_per_tick);
    }
    match state.instance.machine.run(budget) {
        Ok(outcome) => {
            state.outcome = summarize_outcome(outcome);
            if matches!(
                state.outcome,
                RuntimeOutcome::Halted(_) | RuntimeOutcome::Panicked(_)
            ) {
                state.paused = true;
            }
        }
        Err(error) => fault(state, error.to_string()),
    }
    drain_uart(state);
}

fn drain_uart(state: &mut WorkerState) {
    let mut bytes = [0; UART_DRAIN_BYTES];
    loop {
        let drained = state.instance.uart_mut().drain_tx(&mut bytes);
        if drained == 0 {
            break;
        }
        state.terminal.push_guest_bytes(&bytes[..drained]);
    }
}

fn summarize_outcome(outcome: Rv32MachineOutcome) -> RuntimeOutcome {
    match outcome {
        Rv32MachineOutcome::BudgetExhausted { .. } => RuntimeOutcome::BudgetExhausted,
        Rv32MachineOutcome::WaitingForInterrupt { .. } => RuntimeOutcome::WaitingForInterrupt,
        Rv32MachineOutcome::Halted { exit_code, .. } => RuntimeOutcome::Halted(exit_code),
        Rv32MachineOutcome::Panicked { panic_code, .. } => RuntimeOutcome::Panicked(panic_code),
    }
}

fn fault(state: &mut WorkerState, message: String) {
    state.error = Some(message);
    state.outcome = RuntimeOutcome::Faulted;
    state.paused = true;
}

fn publish(state: &mut WorkerState, latest: &LatestSnapshot) {
    state.revision = state.revision.wrapping_add(1);
    latest.publish(RuntimeSnapshot {
        revision: state.revision,
        paused: state.paused,
        mode: state.profile.initial_mode,
        outcome: state.outcome,
        inspection: state.instance.machine.inspection_snapshot(),
        uart: state.instance.uart().diagnostics(),
        terminal: state.terminal.snapshot(),
        error: state.error.clone(),
    });
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),
    #[error(transparent)]
    Build(#[from] compukter_vm::rv32_machine::Rv32MachineBuildError),
    #[error("could not spawn VM worker: {0}")]
    WorkerSpawn(std::io::Error),
    #[error("VM worker has stopped")]
    WorkerStopped,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use compukter_vm::rv32im::encoding::{addi, andi, beq, jal, lbu, lui, sb, sw};

    use super::{RuntimeCommand, RuntimeHandle};
    use crate::profile::{BackendProfile, MachineProfile};

    #[test]
    fn paused_worker_steps_exactly_one_guest_instruction() {
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let elf = machine_program_elf(&[jal(0, 0)]);
        let runtime = RuntimeHandle::spawn(profile, elf).unwrap();
        runtime.command(RuntimeCommand::SetPaused(true)).unwrap();
        let paused = runtime.wait_for(|snapshot| snapshot.paused, Duration::from_secs(1));

        runtime.command(RuntimeCommand::Step).unwrap();
        let stepped = runtime.wait_for(
            |snapshot| {
                snapshot.inspection.hart.retired_instructions
                    > paused.inspection.hart.retired_instructions
            },
            Duration::from_secs(1),
        );

        assert_eq!(
            stepped.inspection.hart.retired_instructions,
            paused.inspection.hart.retired_instructions + 1
        );
        assert!(stepped.paused);
    }

    #[test]
    fn reset_rebuilds_the_machine_and_returns_to_entry_point() {
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let elf = machine_program_elf(&[jal(0, 0)]);
        let runtime = RuntimeHandle::spawn(profile, elf).unwrap();
        runtime.command(RuntimeCommand::SetPaused(true)).unwrap();
        runtime.wait_for(|snapshot| snapshot.paused, Duration::from_secs(1));
        runtime.command(RuntimeCommand::Step).unwrap();
        runtime.wait_for(
            |snapshot| snapshot.inspection.hart.retired_instructions > 0,
            Duration::from_secs(1),
        );

        runtime.command(RuntimeCommand::Reset).unwrap();
        let reset = runtime.wait_for(
            |snapshot| snapshot.inspection.hart.retired_instructions == 0,
            Duration::from_secs(1),
        );

        assert_eq!(reset.inspection.hart.pc, 0x1000);
        assert!(reset.paused);
    }

    #[test]
    fn uart_input_crosses_guest_mmio_and_returns_in_terminal_snapshot() {
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let elf = machine_program_elf(&[
            lui(1, 0x10001),
            lbu(2, 1, 5),
            andi(2, 2, 1),
            beq(2, 0, -8),
            lbu(3, 1, 0),
            sb(1, 3, 0),
            lui(4, 0x10000),
            addi(5, 0, 0),
            sw(4, 5, 8),
            addi(5, 0, 3),
            sw(4, 5, 0),
        ]);
        let runtime = RuntimeHandle::spawn(profile, elf).unwrap();

        runtime
            .command(RuntimeCommand::SendUart(vec![b'Z']))
            .unwrap();
        let echoed = runtime.wait_for(
            |snapshot| snapshot.terminal.raw_bytes.contains(&b'Z'),
            Duration::from_secs(2),
        );

        assert_eq!(echoed.terminal.raw_bytes, b"Z");
    }

    fn machine_program_elf(words: &[u32]) -> Vec<u8> {
        const ELF_HEADER: usize = 52;
        const PROGRAM_HEADER: usize = 32;
        const PAGE: usize = 4096;
        let code: Vec<u8> = words.iter().copied().flat_map(u32::to_le_bytes).collect();
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
