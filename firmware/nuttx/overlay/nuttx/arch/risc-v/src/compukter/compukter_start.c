/* SPDX-License-Identifier: Apache-2.0 */

#include <nuttx/config.h>

#include <stdint.h>

#include <nuttx/init.h>
#include <nuttx/serial/uart_16550.h>

#include "riscv_internal.h"

static void compukter_clear_bss(void)
{
  uint32_t *destination;

  for (destination = (uint32_t *)_sbss;
       destination < (uint32_t *)_ebss;
       destination++)
    {
      *destination = 0;
    }
}

void compukter_start(void)
{
  compukter_clear_bss();

#ifdef USE_EARLYSERIALINIT
  riscv_earlyserialinit();
#endif

  nx_start();

  for (;;)
    {
      __asm__ volatile ("wfi");
    }
}

void riscv_earlyserialinit(void)
{
#ifdef CONFIG_16550_UART
  u16550_earlyserialinit();
#endif
}

void riscv_serialinit(void)
{
#ifdef CONFIG_16550_UART
  u16550_serialinit();
#endif
}
