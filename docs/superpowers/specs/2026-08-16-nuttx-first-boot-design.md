# NuttX First Boot on Compukter-VM

## Objective

Boot an upstream Apache NuttX flat build on Compukter-VM and expose an
interactive NuttShell session through the Playground's existing UART terminal.
The first slice proves the CPU, trap, timer, interrupt, UART, linker, and build
contracts without adding storage, networking, or runtime-loaded applications.

## Scope

The first successful session must provide:

```text
NuttShell (NSH)
nsh> hello
Hello from NuttX on Compukter-VM
nsh>
```

The firmware uses RV32IMA with the ILP32 ABI. Compressed, floating-point, MMU,
S-mode, SMP, semihosting, networking, block storage, and dynamic ELF loading are
out of scope.

## Source and Build Ownership

Compukter-Playground owns a small NuttX overlay under `firmware/nuttx`. It
contains the Compukter board/SoC port, NSH configuration, linker script, and the
`hello` application. It does not vendor or fork NuttX.

A Playground build script accepts or prepares clean upstream `nuttx` and
`nuttx-apps` checkouts in a disposable build directory, applies the overlay,
configures the Compukter NSH target, and writes the resulting ELF to a profile-
relative path. Generated upstream source trees and firmware artifacts remain
untracked.

The initial development build may use checkouts under `/tmp`. Reproducibility
comes from pinning compatible upstream revisions in the build script or a small
version manifest, not from committing the upstream trees.

## Platform Contract

The board port targets one RV32 hart and the existing Playground machine:

- 16 MiB of contiguous guest RAM beginning at guest address zero;
- firmware linked above the low reserved area, with all loadable segments inside
  configured RAM;
- 16550-compatible UART at `0x10001000`;
- UART external interrupt routed through PLIC source 1;
- PLIC machine context at the existing Compukter platform address;
- the existing compact machine timer MMIO interface;
- direct machine-mode boot at the ELF entry point;
- no device tree, SBI, OpenSBI, or QEMU `virt` compatibility assumptions.

The startup code initializes the stack, global pointer, `.bss`, trap vector, and
NuttX board startup state. Timer and external interrupts use the current VM trap
and interrupt semantics. Before adapting around any failure, the implementation
must audit whether NuttX requires a missing architectural CSR or incorrect VM
behavior; architectural gaps belong in Compukter-VM.

## Runtime Design

NuttX runs as a flat build in one machine-mode address space. NSH and `hello`
are built into the firmware. The generic NuttX serial upper half is paired with
a Compukter lower-half implementation backed by the existing 16550 registers.
RX and TX use interrupts rather than terminal polling. Idle execution uses WFI.

The NuttX scheduler tick is driven by the compact machine timer. The port treats
the VM timer's configured units as virtual platform time and rearms the compare
register deterministically. Playground remains responsible for advancing
virtual time and granting the VM its per-tick instruction budget.

## Failure Handling and Diagnostics

Build failures must report the missing host tool or incompatible upstream
revision. Boot diagnostics initially use early polled UART output before the
interrupt-driven serial driver is available. Unexpected traps must preserve and
surface `mcause`, `mepc`, and `mtval` through existing Playground inspection.

The port must not silently substitute QEMU addresses, compressed instructions,
semihosting, or host filesystem access.

## Verification

Verification proceeds in increasing scope:

1. Compile and inspect the ELF architecture, ABI, entry point, program headers,
   W/X permissions, and absence of compressed instructions.
2. Boot headlessly in Compukter-VM and observe the NSH prompt over UART.
3. Send `hello\n` through UART and verify the expected response.
4. Send a second shell command after at least one timer interval to prove timer,
   scheduling, WFI wakeup, PLIC, and UART remain live together.
5. Run the existing Playground library, runtime, and UART firmware tests to
   ensure the original firmware path remains supported.

## Follow-up Slices

After first boot, independent follow-ups may add ROMFS/TMPFS, a persistent block
device, NuttX runtime ELF loading with an exported SDK, and eventually protected
execution. None of those features are prerequisites for accepting this design.
