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

typedef unsigned char u8;
typedef unsigned int u32;

#define MMIO8(address) (*(volatile u8 *)(address))
#define MMIO32(address) (*(volatile u32 *)(address))

#define UART_BASE 0x10001000u
#define UART_RBR MMIO8(UART_BASE + 0u)
#define UART_THR MMIO8(UART_BASE + 0u)
#define UART_IER MMIO8(UART_BASE + 1u)
#define UART_FCR MMIO8(UART_BASE + 2u)
#define UART_LSR MMIO8(UART_BASE + 5u)
#define UART_IER_RX 0x01u
#define UART_IER_TX 0x02u
#define UART_FCR_ENABLE 0x01u
#define UART_LSR_DR 0x01u
#define UART_LSR_THRE 0x20u
#define TX_BUFFER_CAPACITY 256u
#define TX_BUFFER_MASK (TX_BUFFER_CAPACITY - 1u)

#define PLIC_BASE 0x0c000000u
#define PLIC_UART_PRIORITY MMIO32(PLIC_BASE + 4u)
#define PLIC_ENABLE MMIO32(PLIC_BASE + 0x2000u)
#define PLIC_THRESHOLD MMIO32(PLIC_BASE + 0x200000u)
#define PLIC_CLAIM_COMPLETE MMIO32(PLIC_BASE + 0x200004u)
#define UART_SOURCE 1u

#define MSTATUS_MIE (1u << 3)
#define MIE_MEIE (1u << 11)

static const u8 banner[] = "Compukter Playground UART ready\r\n";
static u8 tx_buffer[TX_BUFFER_CAPACITY];
static u32 tx_read;
static u32 tx_write;

static int tx_push(u8 byte) {
    if (tx_write - tx_read == TX_BUFFER_CAPACITY) {
        return 0;
    }
    tx_buffer[tx_write & TX_BUFFER_MASK] = byte;
    ++tx_write;
    return 1;
}

static void uart_service_tx(void) {
    while (tx_read != tx_write && (UART_LSR & UART_LSR_THRE) != 0u) {
        UART_THR = tx_buffer[tx_read & TX_BUFFER_MASK];
        ++tx_read;
    }
    UART_IER = UART_IER_RX | (tx_read != tx_write ? UART_IER_TX : 0u);
}

static void uart_queue(const u8 *bytes, u32 count) {
    for (u32 index = 0; index < count; ++index) {
        (void)tx_push(bytes[index]);
    }
    uart_service_tx();
}

__attribute__((interrupt("machine"), aligned(4))) void machine_trap(void) {
    u32 source = PLIC_CLAIM_COMPLETE;
    if (source == UART_SOURCE) {
        while ((UART_LSR & UART_LSR_DR) != 0u) {
            (void)tx_push(UART_RBR);
        }
        uart_service_tx();
    }
    if (source != 0u) {
        PLIC_CLAIM_COMPLETE = source;
    }
}

__attribute__((noreturn)) void firmware_main(void) {
    UART_IER = 0u;
    UART_FCR = UART_FCR_ENABLE;

    PLIC_UART_PRIORITY = 3u;
    PLIC_ENABLE = 1u << UART_SOURCE;
    PLIC_THRESHOLD = 0u;
    __asm__ volatile("csrw mtvec, %0" : : "r"(machine_trap));
    uart_queue(banner, sizeof(banner) - 1u);
    __asm__ volatile("csrs mie, %0" : : "r"(MIE_MEIE));
    __asm__ volatile("csrs mstatus, %0" : : "r"(MSTATUS_MIE));

    for (;;) {
        __asm__ volatile("wfi");
    }
}
