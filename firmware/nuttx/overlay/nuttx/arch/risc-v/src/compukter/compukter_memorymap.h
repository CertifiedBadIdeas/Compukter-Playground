/* SPDX-License-Identifier: Apache-2.0 */

#ifndef __ARCH_RISCV_SRC_COMPUKTER_COMPUKTER_MEMORYMAP_H
#define __ARCH_RISCV_SRC_COMPUKTER_COMPUKTER_MEMORYMAP_H

#include "riscv_common_memorymap.h"

#ifndef __ASSEMBLY__
#  define COMPUKTER_IDLESTACK_BASE ((uintptr_t)_ebss)
#else
#  define COMPUKTER_IDLESTACK_BASE _ebss
#endif

#endif
