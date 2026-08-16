# NuttX First Boot Implementation Plan

> Issue: [#1](https://github.com/CertifiedBadIdeas/Compukter-Playground/issues/1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and boot a pinned upstream NuttX flat firmware that exposes NSH and a Compukter-specific `hello` command through the existing Playground UART.

**Architecture:** Playground owns a small overlay containing a native NuttX `compukter` RISC-V chip port, `compukter-vm` board, and example application. A build script copies clean pinned NuttX trees into a disposable directory, applies two narrow Kconfig hook patches plus the overlay, and builds with the installed LLVM RISC-V toolchain; the VM and GUI consume the resulting ELF without learning about NuttX.

**Tech Stack:** Apache NuttX and nuttx-apps, RV32IMA/Zicsr/Zifencei ILP32, LLVM 22 (`clang`, `ld.lld`, LLVM binutils), POSIX shell, Rust integration tests, Compukter-VM cached DBT.

---

## File Structure

- `firmware/nuttx/revisions.env`: pinned upstream repository revisions.
- `firmware/nuttx/patches/nuttx-kconfig.patch`: only the architecture and board discovery hooks needed in upstream NuttX.
- `firmware/nuttx/overlay/nuttx/arch/risc-v/include/compukter/irq.h`: NuttX IRQ numbering for PLIC source 1.
- `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/`: startup, trap dispatch, PLIC control, machine timer, heap, and platform constants.
- `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/`: board configuration, linker script, and empty board initialization.
- `firmware/nuttx/overlay/apps/examples/compukter_hello/`: built-in acceptance command.
- `scripts/build-nuttx.sh`: source acquisition, clean disposable assembly, build, and ELF validation.
- `profiles/nuttx.toml`: selectable Playground machine profile for the NuttX artifact.
- `tests/nuttx_firmware.rs`: ignored, explicit end-to-end build/boot/UART acceptance test.
- `README.md`: NuttX build and run instructions.

### Task 1: Pin and validate upstream sources

**Files:**
- Create: `firmware/nuttx/revisions.env`
- Create: `scripts/build-nuttx.sh`
- Test: `scripts/build-nuttx.sh`

- [ ] **Step 1: Write the source-contract checks**

Start `scripts/build-nuttx.sh` with strict shell mode and checks for `git`,
`make`, `clang`, `ld.lld`, `llvm-ar`, `llvm-nm`, `llvm-objcopy`, and
`llvm-readelf`. Accept `NUTTX_SOURCE` and `NUTTX_APPS_SOURCE`; reject a supplied
tree whose `HEAD` differs from the pinned revision. Default to shallow pinned
checkouts below `${TMPDIR:-/tmp}/compukter-playground-nuttx/sources`.

```sh
#!/usr/bin/env sh
set -eu

REPOSITORY=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$REPOSITORY/firmware/nuttx/revisions.env"

for tool in git make clang ld.lld llvm-ar llvm-nm llvm-objcopy llvm-readelf
do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "required NuttX build tool is unavailable: $tool" >&2
    exit 1
  }
done
```

- [ ] **Step 2: Run the script before the overlay exists**

Run:

```sh
NUTTX_SOURCE=/tmp/compukter-nuttx-plan/nuttx \
NUTTX_APPS_SOURCE=/tmp/compukter-nuttx-plan/apps \
./scripts/build-nuttx.sh
```

Expected: FAIL with a clear message that the Compukter overlay is incomplete,
not a compiler or source-version ambiguity.

- [ ] **Step 3: Pin the audited upstream revisions**

Write:

```sh
NUTTX_REV=e9567a7633770d2572638d282d2a575f3895516b
NUTTX_APPS_REV=84ffa84e241f116f514257eb2d0efa7e87470ce7
```

Complete the source preparation logic by copying the two pinned source trees
into a newly created disposable build directory. Never build in or modify a
caller-supplied checkout.

- [ ] **Step 4: Verify deterministic source selection**

Run the command from Step 2 and then repeat it with a checkout at another
revision. Expected: the pinned trees pass source validation; the mismatched
tree fails before copying or compiling.

- [ ] **Step 5: Commit**

```sh
git add firmware/nuttx/revisions.env scripts/build-nuttx.sh
git commit -m "build(nuttx): pin disposable upstream sources (#1)"
```

### Task 2: Register the Compukter architecture and board overlay

**Files:**
- Create: `firmware/nuttx/patches/nuttx-kconfig.patch`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/Kconfig`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/Make.defs`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/chip.h`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/include/compukter/irq.h`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/Kconfig`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/include/board.h`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/src/Makefile`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/src/compukter_boardinit.c`
- Modify: `scripts/build-nuttx.sh`

- [ ] **Step 1: Add a failing overlay-discovery assertion**

After applying the overlay, make the build script assert that generated
configuration can select all four symbols:

```text
CONFIG_ARCH_CHIP_COMPUKTER=y
CONFIG_ARCH_CHIP="compukter"
CONFIG_ARCH_BOARD_COMPUKTER_VM=y
CONFIG_ARCH_BOARD="compukter-vm"
```

Run the script. Expected: FAIL because upstream Kconfig cannot discover the
new chip and board.

- [ ] **Step 2: Add narrow upstream discovery hooks**

Create a patch against the pinned NuttX revision that:

- adds `ARCH_CHIP_COMPUKTER`, selecting `ARCH_RV32`, `ARCH_RV_ISA_M`,
  `ARCH_RV_ISA_A`, `ONESHOT`, `ONESHOT_COUNT`, `ONESHOT_FAST_DIVISION`, and
  `ALARM_ARCH`;
- maps `ARCH_CHIP` to `"compukter"`;
- sources `arch/risc-v/src/compukter/Kconfig`;
- adds `ARCH_BOARD_COMPUKTER_VM` depending on the chip;
- maps `ARCH_BOARD` to `"compukter-vm"`;
- sources the new board Kconfig.

The build script must copy `overlay/nuttx` over the disposable NuttX tree and
apply this patch with `git apply --check` followed by `git apply`.

- [ ] **Step 3: Define the minimal chip and IRQ contract**

`arch/risc-v/include/compukter/irq.h` must contain:

```c
#define COMPUKTER_IRQ_UART0 (RISCV_IRQ_MEXT + 1)
#define NR_IRQS             (COMPUKTER_IRQ_UART0 + 1)
```

`Make.defs` must include `common/Make.defs` and compile exactly:

```make
CHIP_ASRCS = compukter_head.S
CHIP_CSRCS = compukter_start.c compukter_irq_dispatch.c compukter_irq.c
CHIP_CSRCS += compukter_timerisr.c compukter_allocateheap.c
```

`chip.h` includes only the Compukter platform constants and the common RISC-V
internal definitions required by the trap macros. The board has no LEDs or
late device registration; `board_early_initialize()` is an empty function.

- [ ] **Step 4: Generate the configuration**

Run the build script through `tools/configure.sh compukter-vm:nsh`, stopping
before compilation. Expected: `.config` contains the four asserted symbols and
`CONFIG_ARCH_RV32=y`.

- [ ] **Step 5: Commit**

```sh
git add firmware/nuttx/patches firmware/nuttx/overlay scripts/build-nuttx.sh
git commit -m "feat(nuttx): register Compukter RV32 platform (#1)"
```

### Task 3: Add startup, memory layout, and LLVM configuration

**Files:**
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_memorymap.h`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_head.S`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_start.c`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_allocateheap.c`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/hardware/compukter_platform.h`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/hardware/compukter_plic.h`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_irq_dispatch.c`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_irq.c`
- Create: `firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter/compukter_timerisr.c`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/scripts/Make.defs`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/scripts/ld.script`
- Create: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/configs/nsh/defconfig`
- Modify: `scripts/build-nuttx.sh`

- [ ] **Step 1: Make ELF validation fail before a firmware is available**

Append validation that requires ELF32 little-endian RISC-V, entry `0x1000`,
non-W+X PT_LOAD segments, and no `C` extension in the attributes. Run the
script. Expected: FAIL because `nuttx` has not linked.

- [ ] **Step 2: Define startup and heap boundaries**

Link at `0x00001000`; reserve addresses below it; define configured RAM as
`0x00001000..0x01000000`. `compukter_head.S` must set the idle stack with
`riscv_set_inital_sp`, disable `mie`, install `__trap_vec` in `mtvec`, and call
`compukter_start`. `compukter_start.c` must clear `.bss`, initialize the generic
16550 early serial path, and call `nx_start()`. Heap allocation begins at
`g_idle_topstack` and ends at `CONFIG_RAM_END`.

- [ ] **Step 3: Give code and data separate ELF permissions**

The linker script must declare explicit headers:

```ld
PHDRS
{
  text PT_LOAD FLAGS(5);
  data PT_LOAD FLAGS(6);
}
```

Place startup/text/init arrays/rodata/TLS in `text`, page-align before writable
`.data`, and place `.data`/`.bss` in `data`. Export `_stext`, `_etext`, `_sinit`,
`_einit`, `_sdata`, `_edata`, `_sbss`, and `_ebss` as expected by NuttX.

- [ ] **Step 4: Configure a flat LLVM RV32IMA NuttX build**

The defconfig must select the Compukter chip and board, LLVM toolchain,
RV32IMA without C/F/D/V, generic 16550 at `0x10001000`, serial console IRQ 28,
16 MiB physical RAM ending at `0x01000000`, NSH built-ins, and a 50 ms system
tick. It must not enable ELF loading, hostfs, semihosting, SMP, S-mode, MMU, or
networking.

- [ ] **Step 5: Implement PLIC and timer platform adapters**

Use PLIC base `0x0c000000`, machine-context enable `+0x002000`, threshold
`+0x200000`, and claim/complete `+0x200004`. Initialize only source 1, translate
it to `RISCV_IRQ_MEXT + 1`, and claim/complete it around `riscv_doirq()`.

Initialize the common NuttX machine-timer lower half with
`mtime=0x10000208`, `mtimecmp=0x10000200`, IRQ `RISCV_IRQ_MTIMER`, and frequency
20 Hz. Register it with `up_alarm_set_lowerhalf()`. The code and comments must
call this the Compukter compact machine timer, never a CLINT.

- [ ] **Step 6: Build and inspect the ELF**

Run:

```sh
NUTTX_SOURCE=/tmp/compukter-nuttx-plan/nuttx \
NUTTX_APPS_SOURCE=/tmp/compukter-nuttx-plan/apps \
./scripts/build-nuttx.sh profiles/nuttx.elf
llvm-readelf -h -l -A profiles/nuttx.elf
```

Expected: compilation and linking succeed; all load ranges lie below
`0x01000000`; no PT_LOAD is W+X; generated code targets RV32IMA with ILP32.

- [ ] **Step 7: Commit**

```sh
git add firmware/nuttx/overlay scripts/build-nuttx.sh
git commit -m "feat(nuttx): boot flat firmware with LLVM (#1)"
```

### Task 4: Prove traps, scheduling, and interrupt-driven UART at runtime

**Files:**
- Modify only the Compukter port or Compukter-VM files implicated by an observed runtime failure.

- [ ] **Step 1: Boot the startup-only image headlessly**

Run the built ELF in a temporary headless Playground harness with virtual time
advancement. Expected: either early UART progress or an architectural trap;
record `pc`, `mcause`, `mepc`, and `mtval` before changing VM behavior.

- [ ] **Step 2: Verify the single-context PLIC adapter**

Use these exact registers:

```c
#define COMPUKTER_PLIC_BASE      0x0c000000
#define COMPUKTER_PLIC_PRIORITY  (COMPUKTER_PLIC_BASE + 0x000000)
#define COMPUKTER_PLIC_ENABLE    (COMPUKTER_PLIC_BASE + 0x002000)
#define COMPUKTER_PLIC_THRESHOLD (COMPUKTER_PLIC_BASE + 0x200000)
#define COMPUKTER_PLIC_CLAIM     (COMPUKTER_PLIC_BASE + 0x200004)
```

Confirm source 1 is initialized to priority 1 and threshold 0. `up_enable_irq()` and
`up_disable_irq()` translate NuttX external IRQs by subtracting
`RISCV_IRQ_MEXT`. The external dispatcher claims until zero, calls
`riscv_doirq(RISCV_IRQ_MEXT + source, regs)`, and completes the same source.

- [ ] **Step 3: Verify the compact machine timer adapter**

Confirm it passes `mtime=0x10000208`, `mtimecmp=0x10000200`, IRQ `RISCV_IRQ_MTIMER`, and
frequency 20 Hz to `riscv_mtimer_initialize()`, then register the returned
lower half with `up_alarm_set_lowerhalf()`. Do not describe the device as CLINT.

- [ ] **Step 4: Verify interrupt liveness**

Boot while advancing `mtime` by one per simulated 50 ms tick. Expected: NSH
remains schedulable after timer interrupts; UART RX causes PLIC source 1 to be
claimed and completed; idle state reaches WFI without a polling loop.

- [ ] **Step 5: Commit only an evidence-backed correction**

```sh
git add firmware/nuttx/overlay/nuttx/arch/risc-v/src/compukter
git commit -m "fix(nuttx): correct first-boot platform contract (#1)"
```

If runtime verification instead proves a VM architectural defect, test and
commit that correction in Compukter-VM under its VM umbrella issue. If runtime
verification required no correction, skip this commit.

### Task 5: Add the acceptance command and Playground profile

**Files:**
- Create: `firmware/nuttx/overlay/apps/examples/compukter_hello/Kconfig`
- Create: `firmware/nuttx/overlay/apps/examples/compukter_hello/Make.defs`
- Create: `firmware/nuttx/overlay/apps/examples/compukter_hello/Makefile`
- Create: `firmware/nuttx/overlay/apps/examples/compukter_hello/compukter_hello_main.c`
- Create: `profiles/nuttx.toml`
- Modify: `firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/configs/nsh/defconfig`

- [ ] **Step 1: Add the command to the configuration before its source exists**

Enable `CONFIG_EXAMPLES_COMPUKTER_HELLO=y` with program name `hello`, priority
100, and 2048-byte stack. Run the build. Expected: FAIL because the configured
application has no implementation.

- [ ] **Step 2: Implement the built-in command**

Use the normal NuttX application contract:

```c
#include <nuttx/config.h>
#include <stdio.h>

int main(int argc, FAR char *argv[])
{
  printf("Hello from NuttX on Compukter-VM\n");
  return 0;
}
```

Register it through `Application.mk` and the normal generated built-in registry.

- [ ] **Step 3: Add the selectable profile**

Clone the current default machine settings into `profiles/nuttx.toml`, changing
only `firmware.elf` to `nuttx.elf`. Keep 16 MiB RAM, cached DBT with 256 sets,
16-instruction blocks, 100,000 instructions per tick, one timer unit per tick,
and UART base `0x10001000`.

- [ ] **Step 4: Verify interactively**

Run `cargo run --release`, select `nuttx`, and enter `hello`. Expected output:

```text
Hello from NuttX on Compukter-VM
```

- [ ] **Step 5: Commit**

```sh
git add firmware/nuttx/overlay/apps profiles/nuttx.toml \
  firmware/nuttx/overlay/nuttx/boards/risc-v/compukter/compukter-vm/configs/nsh/defconfig
git commit -m "feat(nuttx): expose NSH acceptance firmware (#1)"
```

### Task 6: Automate end-to-end verification and document usage

**Files:**
- Create: `tests/nuttx_firmware.rs`
- Modify: `README.md`
- Modify: `.gitignore`

- [ ] **Step 1: Write the ignored end-to-end test**

Add one deliberately expensive ignored test that invokes `build-nuttx.sh`,
spawns `RuntimeHandle` with the NuttX profile, waits with bounded timeouts for
`nsh>`, sends `hello\n`, waits for the exact banner, sends `help\n` after
additional virtual timer progress, and checks for a second prompt. No source-
text assertions are permitted.

```rust
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use compukter_playground::profile::{MachineProfile, RuntimeMode};
use compukter_playground::runtime::{RuntimeCommand, RuntimeHandle};
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
    let prompt = runtime.wait_for(
        |snapshot| snapshot.terminal.raw_bytes.windows(4).any(|part| part == b"nsh>"),
        Duration::from_secs(10),
    );
    assert!(prompt.error.is_none(), "runtime error: {:?}", prompt.error);

    runtime
        .command(RuntimeCommand::SendUart(b"hello\n".to_vec()))
        .unwrap();
    let hello = runtime.wait_for(
        |snapshot| {
            snapshot
                .terminal
                .raw_bytes
                .windows(b"Hello from NuttX on Compukter-VM".len())
                .any(|part| part == b"Hello from NuttX on Compukter-VM")
        },
        Duration::from_secs(10),
    );
    assert!(hello.error.is_none(), "runtime error: {:?}", hello.error);

    runtime
        .command(RuntimeCommand::SendUart(b"help\n".to_vec()))
        .unwrap();
    let second = runtime.wait_for(
        |snapshot| {
            snapshot
                .terminal
                .raw_bytes
                .windows(b"Builtin Apps:".len())
                .any(|part| part == b"Builtin Apps:")
        },
        Duration::from_secs(10),
    );
    assert!(second.error.is_none(), "runtime error: {:?}", second.error);
}
```

- [ ] **Step 2: Run the test and diagnose only observable failures**

Run:

```sh
cargo test --release --test nuttx_firmware -- --ignored --nocapture
```

Expected: PASS with NSH prompt, exact `hello` output, and a live second command.
If it traps, use the Playground snapshot values rather than patching around the
failure blindly.

- [ ] **Step 3: Run existing regressions**

Run:

```sh
cargo test --lib
cargo test --test uart_firmware
cargo check --all-targets
```

Expected: all existing tests and checks pass.

- [ ] **Step 4: Document and ignore artifacts**

Document the pinned-source build, optional local-source environment variables,
the `nuttx` profile, expected NSH session, and the explicitly ignored acceptance
test. Ignore `profiles/nuttx.elf` and disposable build output, but keep all
overlay sources, patches, revisions, and profile TOML tracked.

- [ ] **Step 5: Commit**

```sh
git add tests/nuttx_firmware.rs README.md .gitignore
git commit -m "test(nuttx): verify NSH first boot end to end (#1)"
```

### Task 7: Final compatibility and scope audit

**Files:**
- Modify only files implicated by verification failures.

- [ ] **Step 1: Inspect final firmware**

Run:

```sh
llvm-readelf -h -l -A profiles/nuttx.elf
llvm-nm -u profiles/nuttx.elf
```

Expected: ELF32 RISC-V ILP32, entry `0x1000`, no W+X PT_LOAD, no unresolved
symbols, no compressed ISA attribute, and all load addresses within RAM.

- [ ] **Step 2: Verify both firmware profiles**

Run the original UART firmware test and the ignored NuttX acceptance test.
Expected: both pass, proving the NuttX addition did not replace the small
firmware workflow.

- [ ] **Step 3: Confirm excluded features stayed excluded**

Inspect the generated `.config`. Expected: no ELF loader, hostfs, semihosting,
networking, MMU, S-mode, SMP, block device, or filesystem storage option beyond
the NuttX pseudo filesystem required for NSH and devices.

- [ ] **Step 4: Commit any verification-only correction**

If verification required a correction, commit only that correction with a
specific message. If no correction was needed, do not create an empty commit.
