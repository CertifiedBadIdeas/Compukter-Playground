# Compukter Playground

Desktop engineering workbench for running and inspecting
[Compukter-VM](https://github.com/CertifiedBadIdeas/Compukter-VM) machines without
starting Minecraft.

The first slice provides:

- an egui/eframe desktop GUI;
- versioned TOML machine profiles with profile-relative ELF paths;
- a dedicated OS thread that exclusively owns the VM;
- realtime 20 TPS and unbounded execution modes;
- pause, single-instruction step, reset, and latest-wins inspection snapshots;
- a 16550-style UART with connect/disconnect and host input;
- ANSI/VT100 and bounded raw-byte terminal views;
- RV32 registers, CSR, timer, PLIC, translation, DBT, and UART diagnostics.

## Repository layout

During development, clone this repository beside `Compukter-VM`:

```text
parent/
├── Compukter-VM/
└── Compukter-Playground/
```

The path dependencies in `Cargo.toml` deliberately use that layout. The
Playground does not depend on the Minecraft mod.

## Linux dependencies

The egui/eframe window uses wgpu and the system X11 or Wayland stack. A working
graphics driver is required; no global-hotkey or `xdotool` dependency is used.

## Run

Build the bundled RV32IM firmware first:

```sh
./scripts/build-firmware.sh
```

This creates the ignored local artifact `profiles/firmware.elf`. The firmware
prints `Compukter Playground UART ready`, drives UART RX and buffered TX through
PLIC interrupts, sleeps with `WFI` while idle, and echoes terminal input.

Then start the workbench:

```sh
cargo run --release
```

The Playground scans direct TOML children of `profiles/`, automatically tries
`profiles/default.toml` (or the first profile alphabetically), and exposes the
catalog through the toolbar selector. The firmware path is resolved relative
to its profile, so each profile and its ELF can be moved together. See
[`profiles/default.toml`](profiles/default.toml).

## Verify

Headless tests do not start a window:

```sh
cargo test --lib
cargo test --test uart_firmware
cargo check --all-targets
```

The runtime tests execute real RV32 ELF bytes and exercise pause/step/reset as
well as UART input through guest MMIO and output back into the terminal
snapshot.

## Runtime model

The egui/UI thread never owns or executes `Rv32Machine`. Commands cross a
bounded channel to `compukter-vm`, a dedicated OS thread. Inspection is copied
into a capacity-one latest-wins mailbox, so a slow GUI cannot create an
unbounded snapshot queue. UART history is bounded separately and therefore
does not disappear when intermediate inspection snapshots are replaced.

Realtime mode advances one configured virtual timer quantum and grants one
instruction budget every 50 ms. Unbounded mode grants the same deterministic
quantum repeatedly without wall-clock pacing. Single-step retires at most one
instruction and does not advance virtual time.
