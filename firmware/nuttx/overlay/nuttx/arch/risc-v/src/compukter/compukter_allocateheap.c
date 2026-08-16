/* SPDX-License-Identifier: Apache-2.0 */

#include <nuttx/config.h>

#include <stddef.h>
#include <stdint.h>

#include <nuttx/arch.h>

#include "riscv_internal.h"

void up_allocate_heap(void **heap_start, size_t *heap_size)
{
  *heap_start = (void *)g_idle_topstack;
  *heap_size = (uintptr_t)CONFIG_RAM_END - g_idle_topstack;
}

#if CONFIG_MM_REGIONS > 1
void riscv_addregion(void)
{
}
#endif
