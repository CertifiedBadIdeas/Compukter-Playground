/* SPDX-License-Identifier: Apache-2.0 */

#include <stdint.h>
#include <stdio.h>

static volatile uint32_t g_compukter_bench_checksum;

int main(int argc, char *argv[])
{
  uint32_t value = 0x9e3779b9u;

  (void)argc;
  (void)argv;

  puts("COMPUKTER BENCH READY");
  fflush(stdout);

  for (;;)
    {
      value ^= value << 13;
      value ^= value >> 17;
      value ^= value << 5;
      value += 0x7f4a7c15u;
      g_compukter_bench_checksum = value;
    }
}
