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

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use compukter_vm::rv32_machine::{
    Rv32DeviceHandle, Rv32Machine, Rv32MachineBuilder, Rv32MachineConfig, Rv32MachineInspection,
    Rv32MachineOutcome,
};
use compukter_vm_devices::virtio::{
    VirtioBlockDevice, VirtioBlockError, VirtioMmioDevice, VirtioTransportError,
};
use compukter_vm_devices::{Uart16550, Uart16550Diagnostics};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError};
use thiserror::Error;

use crate::disk_image::{persist_atomic, DiskImageError, LoadedDiskImage};
use crate::profile::{MachineProfile, RuntimeMode, VIRTIO_BLOCK_BASE};
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
}

#[derive(Debug)]
enum WorkerMessage {
    Command(RuntimeCommand),
    SaveDisk(Sender<Result<PathBuf, RuntimeError>>),
    Shutdown {
        save: bool,
    },
    #[cfg(test)]
    MutateDisk {
        offset: usize,
        value: u8,
        reply: Sender<Result<(), String>>,
    },
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
    pub uart_connected: bool,
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
    messages: Sender<WorkerMessage>,
    latest: Arc<LatestSnapshot>,
    worker: Option<JoinHandle<()>>,
}

impl RuntimeHandle {
    pub fn spawn(
        profile: MachineProfile,
        elf: Vec<u8>,
        disk: Option<LoadedDiskImage>,
    ) -> Result<Self, RuntimeError> {
        profile.validate()?;
        let instance = MachineInstance::build(&profile, &elf, disk)?;
        let (messages, receiver) = bounded(COMMAND_CAPACITY);
        let latest = Arc::new(LatestSnapshot::new());
        let worker_latest = Arc::clone(&latest);
        let worker = thread::Builder::new()
            .name("compukter-vm".to_string())
            .spawn(move || worker_main(profile, elf, instance, receiver, worker_latest))
            .map_err(RuntimeError::WorkerSpawn)?;
        Ok(Self {
            messages,
            latest,
            worker: Some(worker),
        })
    }

    pub fn command(&self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        self.messages
            .send(WorkerMessage::Command(command))
            .map_err(|_| RuntimeError::WorkerStopped)
    }

    pub fn save_disk(&self) -> Result<PathBuf, RuntimeError> {
        let (reply, result) = bounded(1);
        self.messages
            .send(WorkerMessage::SaveDisk(reply))
            .map_err(|_| RuntimeError::WorkerStopped)?;
        result.recv().map_err(|_| RuntimeError::WorkerStopped)?
    }

    #[cfg(test)]
    pub(crate) fn mutate_disk_for_test(&self, offset: usize, value: u8) -> Result<(), String> {
        let (reply, result) = bounded(1);
        self.messages
            .send(WorkerMessage::MutateDisk {
                offset,
                value,
                reply,
            })
            .map_err(|_| "VM worker has stopped".to_string())?;
        result
            .recv()
            .map_err(|_| "VM worker has stopped".to_string())?
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

    pub(crate) fn stop_without_save(mut self) {
        self.stop(false);
    }

    fn stop(&mut self, save: bool) {
        if let Some(worker) = self.worker.take() {
            let _ = self.messages.send(WorkerMessage::Shutdown { save });
            let _ = worker.join();
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.stop(true);
    }
}

struct MachineInstance {
    machine: Rv32Machine,
    uart: Rv32DeviceHandle<Uart16550>,
    disk: Option<AttachedDisk>,
}

type BlockHandle = Rv32DeviceHandle<VirtioMmioDevice<VirtioBlockDevice>>;

struct AttachedDisk {
    path: std::path::PathBuf,
    read_only: bool,
    block: BlockHandle,
}

impl MachineInstance {
    fn build(
        profile: &MachineProfile,
        elf: &[u8],
        disk: Option<LoadedDiskImage>,
    ) -> Result<Self, RuntimeError> {
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
        let (uart, uart_source) = builder.add_mmio_device_with_irq(profile.uart.base, uart);
        debug_assert_eq!(uart_source.get(), 1);
        let disk = if let Some(disk) = disk {
            let (path, bytes, read_only) = disk.into_parts();
            let block = VirtioBlockDevice::from_bytes(bytes, read_only)?;
            let transport = VirtioMmioDevice::new(block)?;
            let (block, block_source) =
                builder.add_mmio_device_with_irq(VIRTIO_BLOCK_BASE, transport);
            debug_assert_eq!(block_source.get(), 2);
            Some(AttachedDisk {
                path,
                read_only,
                block,
            })
        } else {
            None
        };
        let machine = builder.build()?;
        Ok(Self {
            machine,
            uart,
            disk,
        })
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

    fn block(&self) -> Option<&VirtioBlockDevice> {
        self.disk
            .as_ref()
            .and_then(|disk| self.machine.device(disk.block))
            .map(VirtioMmioDevice::device)
    }

    #[cfg(test)]
    fn block_mut(&mut self) -> Option<&mut VirtioBlockDevice> {
        let handle = self.disk.as_ref()?.block;
        self.machine
            .device_mut(handle)
            .map(VirtioMmioDevice::device_mut)
    }

    fn has_writable_disk(&self) -> bool {
        self.disk.as_ref().is_some_and(|disk| !disk.read_only)
    }

    fn cloned_disk_image(&self) -> Option<LoadedDiskImage> {
        let disk = self.disk.as_ref()?;
        Some(LoadedDiskImage::from_parts(
            disk.path.clone(),
            self.block()?.bytes().to_vec(),
            disk.read_only,
        ))
    }
}

struct WorkerState {
    profile: MachineProfile,
    elf: Vec<u8>,
    instance: MachineInstance,
    terminal: TerminalProjection,
    paused: bool,
    outcome: RuntimeOutcome,
    machine_error: Option<String>,
    storage_error: Option<String>,
    revision: u64,
}

fn worker_main(
    profile: MachineProfile,
    elf: Vec<u8>,
    instance: MachineInstance,
    messages: Receiver<WorkerMessage>,
    latest: Arc<LatestSnapshot>,
) {
    let mut state = WorkerState {
        paused: false,
        profile,
        elf,
        instance,
        terminal: TerminalProjection::default(),
        outcome: RuntimeOutcome::Running,
        machine_error: None,
        storage_error: None,
        revision: 0,
    };
    publish(&mut state, &latest);
    let mut next_tick = Instant::now() + REALTIME_TICK;

    loop {
        let message = if state.paused {
            messages.recv().ok()
        } else {
            match state.profile.initial_mode {
                RuntimeMode::Realtime => {
                    let wait = next_tick.saturating_duration_since(Instant::now());
                    match messages.recv_timeout(wait) {
                        Ok(message) => Some(message),
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
                RuntimeMode::Unbounded => match messages.try_recv() {
                    Ok(message) => Some(message),
                    Err(TryRecvError::Empty) => {
                        run_quantum(&mut state, true);
                        publish(&mut state, &latest);
                        continue;
                    }
                    Err(TryRecvError::Disconnected) => None,
                },
            }
        };

        let Some(message) = message else {
            break;
        };
        match message {
            WorkerMessage::Command(command) => handle_command(command, &mut state),
            WorkerMessage::SaveDisk(reply) => {
                let result = save_disk(&mut state);
                publish(&mut state, &latest);
                let _ = reply.send(result);
                next_tick = Instant::now() + REALTIME_TICK;
                continue;
            }
            WorkerMessage::Shutdown { save } => {
                if save && state.instance.has_writable_disk() {
                    if let Err(error) = save_disk(&mut state) {
                        eprintln!("could not save disk during VM shutdown: {error}");
                    }
                }
                break;
            }
            #[cfg(test)]
            WorkerMessage::MutateDisk {
                offset,
                value,
                reply,
            } => {
                let result = state
                    .instance
                    .block_mut()
                    .ok_or_else(|| "no disk is attached".to_string())
                    .and_then(|block| {
                        block
                            .bytes_mut()
                            .get_mut(offset)
                            .ok_or_else(|| format!("disk offset {offset} is out of range"))
                    })
                    .map(|byte| *byte = value);
                let _ = reply.send(result);
            }
        }
        publish(&mut state, &latest);
        next_tick = Instant::now() + REALTIME_TICK;
    }
}

fn handle_command(command: RuntimeCommand, state: &mut WorkerState) {
    match command {
        RuntimeCommand::SetPaused(paused) => state.paused = paused,
        RuntimeCommand::Step => {
            if state.paused {
                run_quantum_with_budget(state, 1, false);
            }
        }
        RuntimeCommand::Reset => match MachineInstance::build(
            &state.profile,
            &state.elf,
            state.instance.cloned_disk_image(),
        ) {
            Ok(instance) => {
                state.instance = instance;
                state.terminal.clear();
                state.outcome = RuntimeOutcome::Running;
                state.machine_error = None;
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
    }
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

fn save_disk(state: &mut WorkerState) -> Result<PathBuf, RuntimeError> {
    let result: Result<PathBuf, RuntimeError> = (|| {
        let disk = state.instance.disk.as_ref().ok_or(RuntimeError::NoDisk)?;
        if disk.read_only {
            return Err(DiskImageError::ReadOnly(disk.path.clone()).into());
        }
        let path = disk.path.clone();
        let bytes = state
            .instance
            .block()
            .expect("runtime block handle invariant")
            .bytes();
        persist_atomic(&path, bytes)?;
        Ok(path)
    })();

    match &result {
        Ok(_) => state.storage_error = None,
        Err(error) => {
            state.storage_error = Some(error.to_string());
            state.paused = true;
        }
    }
    result
}

fn fault(state: &mut WorkerState, message: String) {
    state.machine_error = Some(message);
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
        uart_connected: state.instance.uart().is_connected(),
        uart: state.instance.uart().diagnostics(),
        terminal: state.terminal.snapshot(),
        error: state
            .machine_error
            .as_ref()
            .or(state.storage_error.as_ref())
            .cloned(),
    });
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Profile(#[from] crate::profile::ProfileError),
    #[error(transparent)]
    Build(#[from] compukter_vm::rv32_machine::Rv32MachineBuildError),
    #[error(transparent)]
    Block(#[from] VirtioBlockError),
    #[error(transparent)]
    VirtioTransport(#[from] VirtioTransportError),
    #[error(transparent)]
    Disk(#[from] DiskImageError),
    #[error("no disk is attached")]
    NoDisk,
    #[error("could not spawn VM worker: {0}")]
    WorkerSpawn(std::io::Error),
    #[error("VM worker has stopped")]
    WorkerStopped,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use compukter_vm::rv32im::encoding::{addi, andi, beq, jal, lbu, lui, lw, sb, sw};

    use super::{
        handle_command, MachineInstance, RuntimeCommand, RuntimeError, RuntimeHandle,
        RuntimeOutcome, WorkerState,
    };
    use crate::disk_image::{DiskImageError, LoadedDiskImage};
    use crate::profile::{BackendProfile, DiskProfile, MachineProfile};
    use tempfile::{tempdir, TempDir};

    #[test]
    fn disk_is_mapped_after_uart_with_stable_irq_source() {
        let temporary = tempdir().unwrap();
        fs::write(temporary.path().join("disk.img"), vec![0x5a; 512]).unwrap();
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let disk = LoadedDiskImage::load(
            &temporary.path().join("machine.toml"),
            &DiskProfile {
                image: PathBuf::from("disk.img"),
                read_only: false,
            },
        )
        .unwrap();

        let mut instance = MachineInstance::build(
            &profile,
            &machine_program_elf(&[lui(1, 0x10002), lw(2, 1, 0), jal(0, 0)]),
            Some(disk),
        )
        .unwrap();

        let inspection = instance.machine.inspection_snapshot();
        assert_eq!(inspection.irq_route_count, 2);
        assert_eq!(inspection.irq_routes[0].source, 1);
        assert_eq!(inspection.irq_routes[1].source, 2);
        assert_eq!(instance.block().unwrap().bytes(), &[0x5a; 512]);

        instance.machine.run(2).unwrap();
        assert_eq!(
            instance.machine.inspection_snapshot().hart.registers[2],
            0x7472_6976
        );
    }

    #[test]
    fn reset_preserves_device_bytes_without_saving_the_host_image() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("disk.img");
        fs::write(&path, vec![0x11; 512]).unwrap();
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let disk = LoadedDiskImage::load(
            &temporary.path().join("machine.toml"),
            &DiskProfile {
                image: PathBuf::from("disk.img"),
                read_only: false,
            },
        )
        .unwrap();
        let elf = machine_program_elf(&[jal(0, 0)]);
        let instance = MachineInstance::build(&profile, &elf, Some(disk)).unwrap();
        let mut state = WorkerState {
            profile,
            elf,
            instance,
            terminal: crate::terminal::TerminalProjection::default(),
            paused: true,
            outcome: RuntimeOutcome::Running,
            machine_error: None,
            storage_error: None,
            revision: 0,
        };
        let block = state.instance.disk.as_ref().unwrap().block;
        state
            .instance
            .machine
            .device_mut(block)
            .unwrap()
            .device_mut()
            .bytes_mut()[0] = 0x22;

        handle_command(RuntimeCommand::Reset, &mut state);

        assert_eq!(state.instance.block().unwrap().bytes()[0], 0x22);
        assert_eq!(fs::read(path).unwrap()[0], 0x11);
    }

    #[test]
    fn acknowledged_save_persists_current_device_bytes() {
        let temporary = tempdir().unwrap();
        let (runtime, path) = runtime_with_disk(&temporary, false);
        runtime.mutate_disk_for_test(0, 0x44).unwrap();

        let saved = runtime.save_disk().unwrap();

        assert_eq!(saved, path);
        assert_eq!(fs::read(path).unwrap()[0], 0x44);
    }

    #[test]
    fn failed_save_pauses_but_keeps_the_worker_recoverable() {
        let temporary = tempdir().unwrap();
        let (runtime, path) = runtime_with_disk(&temporary, false);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        assert!(runtime.save_disk().is_err());
        let failed = runtime.wait_for(
            |snapshot| snapshot.paused && snapshot.error.is_some(),
            Duration::from_secs(1),
        );
        assert!(failed.error.as_deref().unwrap().contains("disk image"));

        fs::remove_dir(&path).unwrap();
        runtime.save_disk().unwrap();
        let recovered =
            runtime.wait_for(|snapshot| snapshot.error.is_none(), Duration::from_secs(1));
        assert!(recovered.paused);
    }

    #[test]
    fn read_only_disk_refuses_host_persistence() {
        let temporary = tempdir().unwrap();
        let (runtime, path) = runtime_with_disk(&temporary, true);
        let original = fs::read(&path).unwrap();

        assert!(matches!(
            runtime.save_disk(),
            Err(RuntimeError::Disk(DiskImageError::ReadOnly(_)))
        ));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn dropping_runtime_saves_before_the_worker_is_joined() {
        let temporary = tempdir().unwrap();
        let (runtime, path) = runtime_with_disk(&temporary, false);
        runtime.mutate_disk_for_test(0, 0x77).unwrap();

        drop(runtime);

        assert_eq!(fs::read(path).unwrap()[0], 0x77);
    }

    #[test]
    fn paused_worker_steps_exactly_one_guest_instruction() {
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let elf = machine_program_elf(&[jal(0, 0)]);
        let runtime = RuntimeHandle::spawn(profile, elf, None).unwrap();
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
        let runtime = RuntimeHandle::spawn(profile, elf, None).unwrap();
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
        let runtime = RuntimeHandle::spawn(profile, elf, None).unwrap();

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

    fn runtime_with_disk(temporary: &TempDir, read_only: bool) -> (RuntimeHandle, PathBuf) {
        let path = temporary.path().join("disk.img");
        fs::write(&path, vec![0x11; 512]).unwrap();
        let disk = LoadedDiskImage::load(
            &temporary.path().join("machine.toml"),
            &DiskProfile {
                image: PathBuf::from("disk.img"),
                read_only,
            },
        )
        .unwrap();
        let mut profile = MachineProfile::default();
        profile.machine.backend = BackendProfile::Cached { sets: 16 };
        let runtime =
            RuntimeHandle::spawn(profile, machine_program_elf(&[jal(0, 0)]), Some(disk)).unwrap();
        (runtime, path)
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
