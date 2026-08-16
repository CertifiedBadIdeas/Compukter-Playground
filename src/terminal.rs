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

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TerminalMode {
    #[default]
    Ansi,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub mode: TerminalMode,
    pub ansi_text: String,
    pub raw_bytes: Vec<u8>,
}

pub struct TerminalProjection {
    parser: vt100::Parser,
    rows: u16,
    columns: u16,
    raw_capacity: usize,
    raw_bytes: VecDeque<u8>,
    mode: TerminalMode,
}

impl std::fmt::Debug for TerminalProjection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalProjection")
            .field("rows", &self.rows)
            .field("columns", &self.columns)
            .field("raw_capacity", &self.raw_capacity)
            .field("raw_len", &self.raw_bytes.len())
            .field("mode", &self.mode)
            .finish()
    }
}

impl Default for TerminalProjection {
    fn default() -> Self {
        Self::new(30, 100, 64 * 1024)
    }
}

impl TerminalProjection {
    pub fn new(rows: u16, columns: u16, raw_capacity: usize) -> Self {
        assert!(rows > 0);
        assert!(columns > 0);
        Self {
            parser: vt100::Parser::new(rows, columns, 1_000),
            rows,
            columns,
            raw_capacity,
            raw_bytes: VecDeque::with_capacity(raw_capacity),
            mode: TerminalMode::Ansi,
        }
    }

    pub fn push_guest_bytes(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        let overflow = self
            .raw_bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.raw_capacity);
        self.raw_bytes.drain(..overflow.min(self.raw_bytes.len()));
        let skip = bytes.len().saturating_sub(self.raw_capacity);
        self.raw_bytes.extend(bytes[skip..].iter().copied());
    }

    pub fn set_mode(&mut self, mode: TerminalMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> TerminalMode {
        self.mode
    }

    pub fn clear(&mut self) {
        self.parser = vt100::Parser::new(self.rows, self.columns, 1_000);
        self.raw_bytes.clear();
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot {
            mode: self.mode,
            ansi_text: self.parser.screen().contents(),
            raw_bytes: self.raw_bytes.iter().copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TerminalMode, TerminalProjection};

    #[test]
    fn ansi_and_raw_views_share_the_same_uart_stream() {
        let mut terminal = TerminalProjection::new(4, 20, 64);
        terminal.push_guest_bytes(b"hello\rworld\n\x1b[31mred\x1b[0m");

        let snapshot = terminal.snapshot();
        assert!(snapshot.ansi_text.contains("world"));
        assert!(snapshot.ansi_text.contains("red"));
        assert_eq!(snapshot.raw_bytes, b"hello\rworld\n\x1b[31mred\x1b[0m");

        terminal.set_mode(TerminalMode::Raw);
        assert_eq!(terminal.snapshot().mode, TerminalMode::Raw);
        assert!(terminal.snapshot().ansi_text.contains("red"));
    }

    #[test]
    fn raw_history_is_bounded_to_the_newest_bytes() {
        let mut terminal = TerminalProjection::new(2, 8, 4);
        terminal.push_guest_bytes(&[1, 2, 3]);
        terminal.push_guest_bytes(&[4, 5, 6]);

        assert_eq!(terminal.snapshot().raw_bytes, [3, 4, 5, 6]);
    }

    #[test]
    fn clear_resets_both_projections() {
        let mut terminal = TerminalProjection::new(2, 8, 8);
        terminal.push_guest_bytes(b"text");
        terminal.clear();

        let snapshot = terminal.snapshot();
        assert!(snapshot.ansi_text.trim().is_empty());
        assert!(snapshot.raw_bytes.is_empty());
    }
}
