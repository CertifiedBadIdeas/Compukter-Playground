/* SPDX-License-Identifier: Apache-2.0 */

#include <nuttx/config.h>

#include <stdint.h>

#include <nuttx/arch.h>
#include <nuttx/timers/arch_alarm.h>

#include "riscv_internal.h"
#include "riscv_mtimer.h"
#include "hardware/compukter_platform.h"

void up_timer_initialize(void)
{
  struct oneshot_lowerhalf_s *lower;

  lower = riscv_mtimer_initialize(COMPUKTER_TIMER_MTIME,
                                  COMPUKTER_TIMER_MTIMECMP,
                                  RISCV_IRQ_TIMER,
                                  COMPUKTER_TIMER_FREQUENCY);
  DEBUGASSERT(lower != NULL);
  up_alarm_set_lowerhalf(lower);
}
