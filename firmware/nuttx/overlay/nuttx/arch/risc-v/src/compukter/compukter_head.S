/* SPDX-License-Identifier: Apache-2.0 */

#include <nuttx/config.h>

#include <arch/arch.h>
#include <arch/irq.h>

#include "chip.h"
#include "riscv_internal.h"
#include "riscv_macros.S"

  .section .text
  .global __start
  .type __start, function

__start:
  riscv_set_inital_sp COMPUKTER_IDLESTACK_BASE, SMP_STACK_SIZE, zero
  csrw CSR_IE, zero
  la t0, __trap_vec
  csrw CSR_TVEC, t0
  call compukter_start

1:
  wfi
  j 1b

  .global _init
  .global _fini

_init:
_fini:
  ret
