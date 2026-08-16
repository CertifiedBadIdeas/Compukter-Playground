/*
 * The Compukter Kraft Developers
 *
 * Copyright (C) 2026 Vsevolod Petrov (lazyhat)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 */

    .option norvc
    .section .text.start, "ax", @progbits
    .globl _start
    .type _start, @function
_start:
    .option push
    .option norelax
    la sp, __stack_top
    la t0, __bss_start
    la t1, __bss_end
    .option pop

1:
    bgeu t0, t1, 2f
    sw zero, 0(t0)
    addi t0, t0, 4
    j 1b

2:
    call firmware_main

3:
    wfi
    j 3b
    .size _start, . - _start
