use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Instant;

use compukter_playground::profile::MachineProfile;
use compukter_vm::rv32_machine::{
    Rv32DeviceHandle, Rv32Machine, Rv32MachineBuilder, Rv32MachineConfig, Rv32MachineOutcome,
};
use compukter_vm_devices::Uart16550;

const READY: &[u8] = b"COMPUKTER BENCH READY";
const PROMPT: &[u8] = b"nsh>";
const BOOT_TICK_LIMIT: usize = 2_000;
const WARMUP_TICKS: usize = 5;

struct Guest {
    machine: Rv32Machine,
    uart: Rv32DeviceHandle<Uart16550>,
    output: Vec<u8>,
    command_sent: bool,
    ready: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let population = arguments
        .next()
        .unwrap_or_else(|| "100".to_string())
        .parse::<usize>()?;
    let measured_ticks = arguments
        .next()
        .unwrap_or_else(|| "10".to_string())
        .parse::<usize>()?;
    if population == 0 || measured_ticks == 0 {
        return Err("population and measured tick count must be positive".into());
    }

    let profile_path = Path::new("profiles/nuttx-bench.toml");
    let profile = MachineProfile::load(profile_path)?;
    let elf = fs::read(profile.resolve_firmware(profile_path))?;

    let cold_started = Instant::now();
    let mut guests = Vec::with_capacity(population);
    for _ in 0..population {
        guests.push(build_guest(&profile, &elf)?);
    }
    let construction_seconds = cold_started.elapsed().as_secs_f64();

    let boot_started = Instant::now();
    let mut boot_ticks = 0;
    while guests.iter().any(|guest| !guest.ready) {
        if boot_ticks == BOOT_TICK_LIMIT {
            let ready = guests.iter().filter(|guest| guest.ready).count();
            return Err(format!("only {ready}/{population} NuttX guests became ready").into());
        }
        for guest in &mut guests {
            if guest.ready {
                continue;
            }
            run_guest(guest, &profile)?;
            drain_boot_uart(guest);
            if !guest.command_sent && contains(&guest.output, PROMPT) {
                guest
                    .machine
                    .device_mut(guest.uart)
                    .unwrap()
                    .inject_rx(b"bench\n");
                guest.command_sent = true;
            }
            if contains(&guest.output, READY) {
                guest.ready = true;
                guest.output.clear();
            }
        }
        boot_ticks += 1;
    }
    let boot_seconds = boot_started.elapsed().as_secs_f64();

    for _ in 0..WARMUP_TICKS {
        for guest in &mut guests {
            let _ = run_guest(guest, &profile)?;
            drain_discard(guest);
        }
    }

    let cpu_before = process_cpu_seconds()?;
    let measured_started = Instant::now();
    let mut tick_seconds = Vec::with_capacity(measured_ticks);
    let mut retired = 0_u64;
    for _ in 0..measured_ticks {
        let tick_started = Instant::now();
        for guest in &mut guests {
            retired = retired.saturating_add(run_guest(guest, &profile)?);
            drain_discard(guest);
        }
        tick_seconds.push(tick_started.elapsed().as_secs_f64());
    }
    let wall_seconds = measured_started.elapsed().as_secs_f64();
    let cpu_seconds = process_cpu_seconds()? - cpu_before;

    tick_seconds.sort_by(f64::total_cmp);
    let average_tick = tick_seconds.iter().sum::<f64>() / tick_seconds.len() as f64;
    let p50_tick = percentile(&tick_seconds, 0.50);
    let p95_tick = percentile(&tick_seconds, 0.95);
    let max_tick = *tick_seconds.last().unwrap();
    let realtime_factor = 0.050 / average_tick;
    let estimated_capacity = population as f64 * realtime_factor;
    let guest_mips = retired as f64 / wall_seconds / 1_000_000.0;
    let cpu_percent = cpu_seconds / wall_seconds * 100.0;
    let (rss_kib, peak_rss_kib) = memory_kib()?;

    println!("population,ticks,budget,construct_s,boot_s,boot_ticks,avg_tick_ms,p50_tick_ms,p95_tick_ms,max_tick_ms,realtime_factor,estimated_capacity,guest_mips,cpu_percent,rss_mib,peak_rss_mib,retired");
    println!(
        "{population},{measured_ticks},{},{construction_seconds:.6},{boot_seconds:.6},{boot_ticks},{:.6},{:.6},{:.6},{:.6},{realtime_factor:.6},{estimated_capacity:.1},{guest_mips:.3},{cpu_percent:.2},{:.2},{:.2},{retired}",
        profile.clock.instructions_per_tick,
        average_tick * 1_000.0,
        p50_tick * 1_000.0,
        p95_tick * 1_000.0,
        max_tick * 1_000.0,
        rss_kib as f64 / 1024.0,
        peak_rss_kib as f64 / 1024.0,
    );

    Ok(())
}

fn build_guest(profile: &MachineProfile, elf: &[u8]) -> Result<Guest, Box<dyn Error>> {
    let config = Rv32MachineConfig {
        ram_size: profile.machine.ram_bytes,
        debug_limit: profile.machine.debug_limit,
        execution: profile.machine.backend.execution_config(),
    };
    let mut uart = Uart16550::new();
    uart.connect();
    let mut builder = Rv32MachineBuilder::from_elf(elf, config)?;
    let (uart, _) = builder.add_mmio_device_with_irq(profile.uart.base, uart);
    Ok(Guest {
        machine: builder.build()?,
        uart,
        output: Vec::with_capacity(1024),
        command_sent: false,
        ready: false,
    })
}

fn run_guest(guest: &mut Guest, profile: &MachineProfile) -> Result<u64, Box<dyn Error>> {
    guest
        .machine
        .advance_time(profile.clock.timer_units_per_tick);
    match guest.machine.run(profile.clock.instructions_per_tick)? {
        Rv32MachineOutcome::BudgetExhausted { retired_delta, .. } => Ok(retired_delta),
        Rv32MachineOutcome::WaitingForInterrupt { retired_delta, .. } if !guest.ready => {
            Ok(retired_delta)
        }
        outcome => {
            Err(format!("saturated NuttX guest stopped consuming its budget: {outcome:?}").into())
        }
    }
}

fn drain_boot_uart(guest: &mut Guest) {
    let mut bytes = [0_u8; 256];
    loop {
        let count = guest
            .machine
            .device_mut(guest.uart)
            .unwrap()
            .drain_tx(&mut bytes);
        if count == 0 {
            break;
        }
        guest.output.extend_from_slice(&bytes[..count]);
    }
    if guest.output.len() > 4096 {
        let keep_from = guest.output.len() - 4096;
        guest.output.drain(..keep_from);
    }
}

fn drain_discard(guest: &mut Guest) {
    let mut bytes = [0_u8; 256];
    while guest
        .machine
        .device_mut(guest.uart)
        .unwrap()
        .drain_tx(&mut bytes)
        != 0
    {}
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|part| part == needle)
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    let index = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted[index]
}

fn process_cpu_seconds() -> Result<f64, Box<dyn Error>> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let result = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(time.tv_sec as f64 + time.tv_nsec as f64 / 1_000_000_000.0)
}

fn memory_kib() -> Result<(u64, u64), Box<dyn Error>> {
    let status = fs::read_to_string("/proc/self/status")?;
    let mut rss = None;
    let mut peak = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss = value
                .split_whitespace()
                .next()
                .and_then(|text| text.parse().ok());
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            peak = value
                .split_whitespace()
                .next()
                .and_then(|text| text.parse().ok());
        }
    }
    Ok((
        rss.ok_or("VmRSS is missing")?,
        peak.ok_or("VmHWM is missing")?,
    ))
}
