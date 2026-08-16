/* SPDX-License-Identifier: Apache-2.0 */

#include <nuttx/config.h>

#include <stdint.h>

#include <nuttx/arch.h>
#include <nuttx/irq.h>

#include "riscv_internal.h"
#include "hardware/compukter_plic.h"

static uintreg_t *compukter_dispatch_external(uintreg_t *regs)
{
  int source;

  while ((source = getreg32(COMPUKTER_PLIC_CLAIM)) != 0)
    {
      regs = riscv_doirq(RISCV_IRQ_EXT + source, regs);
      putreg32(source, COMPUKTER_PLIC_CLAIM);
    }

  return regs;
}

void *riscv_dispatch_irq(uintreg_t vector, uintreg_t *regs)
{
  int irq = vector & ~RISCV_IRQ_BIT;

  if ((vector & RISCV_IRQ_BIT) != 0)
    {
      irq += RISCV_IRQ_ASYNC;
    }

  if (irq == RISCV_IRQ_EXT)
    {
      regs = compukter_dispatch_external(regs);
    }
  else
    {
      regs = riscv_doirq(irq, regs);
    }

  return regs;
}
