/* SPDX-License-Identifier: Apache-2.0 */

#include <nuttx/config.h>

#include <assert.h>
#include <stdint.h>

#include <nuttx/arch.h>
#include <nuttx/irq.h>

#include "riscv_internal.h"
#include "hardware/compukter_platform.h"
#include "hardware/compukter_plic.h"

void up_irqinitialize(void)
{
  up_irq_save();
  putreg32(0, COMPUKTER_PLIC_ENABLE);
  putreg32(1, COMPUKTER_PLIC_PRIORITY + 4 * COMPUKTER_UART0_SOURCE);
  putreg32(0, COMPUKTER_PLIC_THRESHOLD);

#if defined(CONFIG_STACK_COLORATION) && CONFIG_ARCH_INTERRUPTSTACK > 15
  riscv_stack_color(g_intstackalloc, CONFIG_ARCH_INTERRUPTSTACK & ~15);
#endif

  riscv_exception_attach();

#ifndef CONFIG_SUPPRESS_INTERRUPTS
  riscv_color_intstack();
  up_irq_enable();
#endif
}

void up_disable_irq(int irq)
{
  if (irq == RISCV_IRQ_SOFT)
    {
      CLEAR_CSR(CSR_IE, IE_SIE);
    }
  else if (irq == RISCV_IRQ_TIMER)
    {
      CLEAR_CSR(CSR_IE, IE_TIE);
    }
  else if (irq == RISCV_IRQ_EXT)
    {
      CLEAR_CSR(CSR_IE, IE_EIE);
    }
  else if (irq > RISCV_IRQ_EXT)
    {
      int source = irq - RISCV_IRQ_EXT;
      DEBUGASSERT(source == COMPUKTER_UART0_SOURCE);
      modifyreg32(COMPUKTER_PLIC_ENABLE, 1u << source, 0);
    }
}

void up_enable_irq(int irq)
{
  if (irq == RISCV_IRQ_SOFT)
    {
      SET_CSR(CSR_IE, IE_SIE);
    }
  else if (irq == RISCV_IRQ_TIMER)
    {
      SET_CSR(CSR_IE, IE_TIE);
    }
  else if (irq == RISCV_IRQ_EXT)
    {
      SET_CSR(CSR_IE, IE_EIE);
    }
  else if (irq > RISCV_IRQ_EXT)
    {
      int source = irq - RISCV_IRQ_EXT;
      DEBUGASSERT(source == COMPUKTER_UART0_SOURCE);
      modifyreg32(COMPUKTER_PLIC_ENABLE, 0, 1u << source);
    }
}

irqstate_t up_irq_enable(void)
{
  irqstate_t previous;

  up_enable_irq(RISCV_IRQ_EXT);
  previous = READ_AND_SET_CSR(CSR_STATUS, STATUS_IE);
  return previous;
}
