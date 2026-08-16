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

Building the NuttX firmware additionally requires Git, Make, LLVM/Clang with
`ld.lld` and LLVM binutils, `riscv64-elf-gcc` for its RV32 `libgcc` multilib,
and `kconfig-frontends` (`kconfig-conf` plus `kconfig-tweak`). If the Kconfig
tools are installed outside `PATH`, set `NUTTX_KCONFIG_BIN` to their `bin`
directory.

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

## NuttX

Build the pinned NuttX and nuttx-apps revisions into the ignored profile-local
artifact:

```sh
./scripts/build-nuttx.sh profiles/nuttx.elf
```

The script uses shallow cached checkouts and always assembles a disposable
source tree before applying the tracked Compukter overlay. For an offline build,
`NUTTX_SOURCE` and `NUTTX_APPS_SOURCE` may point to clean checkouts at the exact
revisions recorded in `firmware/nuttx/revisions.env`; supplied mismatched or
dirty trees are rejected.

Start the Playground, select `nuttx.toml`, and use its UART terminal. A working
session looks like:

```text
NuttShell (NSH)
nsh> hello
Hello from NuttX on Compukter-VM
nsh>
```

This first port is a flat RV32IMA/ILP32 firmware with NSH, the compact machine
timer, PLIC source 1, and interrupt-driven 16550 UART. It intentionally does not
enable an MMU, networking, block storage, or a loadable-program format yet.

## Verify

Headless tests do not start a window:

```sh
cargo test --lib
cargo test --test uart_firmware
cargo check --all-targets
```

The complete NuttX build-and-boot acceptance test is intentionally ignored in
normal runs because it rebuilds both pinned upstream trees:

```sh
cargo test --release --test nuttx_firmware -- --ignored --nocapture
```

It waits for NSH, runs `hello`, then runs `help` in the same UART session while
also checking WFI, timer, and PLIC liveness.

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
