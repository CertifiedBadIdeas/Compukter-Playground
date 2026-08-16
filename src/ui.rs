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

use std::fmt::Write as _;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};

use compukter_playground::profile::RuntimeMode;
use compukter_playground::runtime::{RuntimeCommand, RuntimeSnapshot};
use compukter_playground::terminal::TerminalMode;
use compukter_playground::view_model::PlaygroundViewModel;

const REFRESH_INTERVAL: Duration = Duration::from_millis(50);
const BACKGROUND: Color32 = Color32::from_rgb(9, 12, 17);
const PANEL: Color32 = Color32::from_rgb(19, 24, 32);
const TERMINAL: Color32 = Color32::from_rgb(4, 5, 8);
const BORDER: Color32 = Color32::from_rgb(46, 56, 74);
const TEXT: Color32 = Color32::from_rgb(224, 232, 245);
const MUTED: Color32 = Color32::from_rgb(140, 158, 184);
const BUTTON: Color32 = Color32::from_rgb(23, 50, 77);
const BUTTON_HOVER: Color32 = Color32::from_rgb(35, 71, 102);

pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 640.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Compukter Playground",
        options,
        Box::new(|creation| Ok(Box::new(PlaygroundApp::new(creation)))),
    )
}

#[derive(Debug, Default)]
struct UartInputState {
    text: String,
}

impl UartInputState {
    fn take_submission(&mut self) -> Option<Vec<u8>> {
        if self.text.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.text).into_bytes())
        }
    }
}

struct PlaygroundApp {
    view_model: PlaygroundViewModel,
    uart_input: UartInputState,
}

impl PlaygroundApp {
    fn new(creation: &eframe::CreationContext<'_>) -> Self {
        configure_style(&creation.egui_ctx);
        Self {
            view_model: PlaygroundViewModel::from_profiles_dir("profiles"),
            uart_input: UartInputState::default(),
        }
    }

    fn command(&mut self, command: RuntimeCommand) {
        if let Err(error) = self.view_model.command(command) {
            self.view_model.set_status_error(error);
        }
    }

    fn submit_uart(&mut self) {
        if let Some(bytes) = self.uart_input.take_submission() {
            self.command(RuntimeCommand::SendUart(bytes));
        }
    }

    fn toolbar(&mut self, root: &mut egui::Ui, snapshot: Option<&RuntimeSnapshot>) {
        egui::Panel::top("toolbar")
            .frame(panel_frame().inner_margin(egui::Margin::symmetric(10, 7)))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Compukter Playground").color(TEXT));
                    ui.separator();
                    self.profile_selector(ui);

                    if ui.button("Save").clicked() {
                        if let Err(error) = self.view_model.save_profile() {
                            self.view_model.set_status_error(error);
                        }
                    }

                    let paused = snapshot.is_none_or(|snapshot| snapshot.paused);
                    if ui.button(if paused { "Run" } else { "Pause" }).clicked() {
                        self.command(RuntimeCommand::SetPaused(!paused));
                    }
                    if ui.button("Step").clicked() {
                        self.command(RuntimeCommand::Step);
                    }
                    if ui.button("Reset").clicked() {
                        self.command(RuntimeCommand::Reset);
                    }

                    let mode = snapshot.map(|snapshot| snapshot.mode);
                    let mode_label = match mode {
                        Some(RuntimeMode::Unbounded) => "Unbounded",
                        _ => "Realtime 20 TPS",
                    };
                    if ui.button(mode_label).clicked() {
                        let next = match mode {
                            Some(RuntimeMode::Unbounded) => RuntimeMode::Realtime,
                            _ => RuntimeMode::Unbounded,
                        };
                        self.command(RuntimeCommand::SetMode(next));
                    }
                });
            });
    }

    fn profile_selector(&mut self, ui: &mut egui::Ui) {
        let active = self
            .view_model
            .profile_path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Select profile".to_string());
        let profiles: Vec<String> = self
            .view_model
            .profiles()
            .iter()
            .map(|entry| entry.name().to_owned())
            .collect();
        let mut selected = None;

        egui::ComboBox::from_id_salt("profile-selector")
            .selected_text(format!("Profile: {active}"))
            .show_ui(ui, |ui| {
                for profile in profiles {
                    if ui.selectable_label(profile == active, &profile).clicked() {
                        selected = Some(profile);
                    }
                }
            });

        if let Some(profile) = selected {
            if profile != active {
                if let Err(error) = self.view_model.select_profile(&profile) {
                    self.view_model.set_status_error(error);
                }
            }
        }
    }

    fn inspector(&self, root: &mut egui::Ui, snapshot: Option<&RuntimeSnapshot>) {
        egui::Panel::right("inspector")
            .default_size(440.0)
            .min_size(320.0)
            .resizable(true)
            .frame(panel_frame().inner_margin(egui::Margin::same(8)))
            .show(root, |ui| {
                let (registers, platform, stats) = snapshot.map_or_else(
                    || {
                        (
                            "No register state".to_string(),
                            "No platform state".to_string(),
                            "No runtime statistics".to_string(),
                        )
                    },
                    |snapshot| {
                        (
                            format_registers(snapshot),
                            format_platform(snapshot),
                            format_statistics(snapshot),
                        )
                    },
                );
                info_card(ui, "Registers", &registers);
                ui.add_space(6.0);
                info_card(ui, "CSR / timer / PLIC", &platform);
                ui.add_space(6.0);
                info_card(ui, "Execution", &stats);
            });
    }

    fn terminal(&mut self, root: &mut egui::Ui, snapshot: Option<&RuntimeSnapshot>) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BACKGROUND)
                    .inner_margin(egui::Margin::same(8)),
            )
            .show(root, |ui| {
                let terminal_mode = snapshot
                    .map(|snapshot| snapshot.terminal.mode)
                    .unwrap_or(TerminalMode::Ansi);
                let connected = snapshot.is_some_and(|snapshot| snapshot.uart_connected);

                ui.horizontal(|ui| {
                    ui.heading(RichText::new("UART terminal").color(TEXT));
                    let terminal_label = match terminal_mode {
                        TerminalMode::Ansi => "ANSI / VT100",
                        TerminalMode::Raw => "Raw bytes",
                    };
                    if ui.button(terminal_label).clicked() {
                        let next = match terminal_mode {
                            TerminalMode::Ansi => TerminalMode::Raw,
                            TerminalMode::Raw => TerminalMode::Ansi,
                        };
                        self.command(RuntimeCommand::SetTerminalMode(next));
                    }
                    if ui
                        .button(if connected { "Disconnect" } else { "Connect" })
                        .clicked()
                    {
                        self.command(RuntimeCommand::SetUartConnected(!connected));
                    }
                    if ui.button("Clear").clicked() {
                        self.command(RuntimeCommand::ClearTerminal);
                    }
                });
                ui.add_space(4.0);

                let output = snapshot.map_or_else(
                    || "Open a TOML machine profile to start the VM.".to_string(),
                    |snapshot| match terminal_mode {
                        TerminalMode::Ansi => snapshot.terminal.ansi_text.clone(),
                        TerminalMode::Raw => format_raw(&snapshot.terminal.raw_bytes),
                    },
                );
                let input_height = 34.0;
                egui::Frame::new()
                    .fill(TERMINAL)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .max_height((ui.available_height() - input_height).max(80.0))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(output).monospace().color(TEXT))
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            });
                    });
                ui.add_space(6.0);

                let mut submit = false;
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 64.0).max(120.0);
                    let response = ui.add_sized(
                        [width, 28.0],
                        egui::TextEdit::singleline(&mut self.uart_input.text)
                            .hint_text("UART input")
                            .font(egui::TextStyle::Monospace),
                    );
                    submit |= response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    submit |= ui
                        .add_sized([58.0, 28.0], egui::Button::new("Send"))
                        .clicked();
                });
                if submit {
                    self.submit_uart();
                }
            });
    }

    fn status_bar(&self, root: &mut egui::Ui, snapshot: Option<&RuntimeSnapshot>) {
        egui::Panel::bottom("status")
            .frame(panel_frame().inner_margin(egui::Margin::symmetric(10, 5)))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let status = snapshot.map_or_else(
                        || "Stopped".to_string(),
                        |snapshot| {
                            format!(
                                "{:?} | {:?} | revision {}",
                                snapshot.outcome, snapshot.mode, snapshot.revision
                            )
                        },
                    );
                    ui.label(RichText::new(status).color(MUTED));
                    let message = snapshot
                        .and_then(|snapshot| snapshot.error.as_deref())
                        .or_else(|| self.view_model.status_message())
                        .unwrap_or("");
                    ui.label(RichText::new(message).color(TEXT));
                });
            });
    }
}

impl eframe::App for PlaygroundApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(REFRESH_INTERVAL);
        let snapshot = self.view_model.snapshot();
        self.toolbar(ui, snapshot.as_deref());
        self.status_bar(ui, snapshot.as_deref());
        self.inspector(ui, snapshot.as_deref());
        self.terminal(ui, snapshot.as_deref());
    }
}

fn configure_style(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = TERMINAL;
    visuals.faint_bg_color = BACKGROUND;
    visuals.widgets.inactive.bg_fill = BUTTON;
    visuals.widgets.inactive.weak_bg_fill = BUTTON;
    visuals.widgets.hovered.bg_fill = BUTTON_HOVER;
    visuals.widgets.active.bg_fill = BUTTON_HOVER;
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.widgets.hovered.fg_stroke.color = TEXT;
    visuals.widgets.active.fg_stroke.color = TEXT;
    visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    context.set_visuals(visuals);

    context.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}

fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0, BORDER))
}

fn info_card(ui: &mut egui::Ui, title: &str, contents: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(28, 35, 47))
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.label(RichText::new(title).size(15.0).color(TEXT));
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(RichText::new(contents).monospace().color(MUTED))
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
        });
}

fn format_raw(bytes: &[u8]) -> String {
    let mut output = String::new();
    for (line, chunk) in bytes.chunks(16).enumerate() {
        let _ = write!(output, "{:08x}  ", line * 16);
        for byte in chunk {
            let _ = write!(output, "{byte:02x} ");
        }
        for _ in chunk.len()..16 {
            output.push_str("   ");
        }
        output.push_str(" |");
        for byte in chunk {
            output.push(if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            });
        }
        output.push_str("|\n");
    }
    output
}

fn format_registers(snapshot: &RuntimeSnapshot) -> String {
    let hart = &snapshot.inspection.hart;
    let mut output = format!(
        "pc  {:#010x}\ninstret {}\nWFI {}\n\n",
        hart.pc, hart.retired_instructions, hart.waiting_for_interrupt
    );
    for row in 0..8 {
        for column in 0..4 {
            let index = row + column * 8;
            let _ = write!(output, "x{index:02} {:08x}  ", hart.registers[index]);
        }
        output.push('\n');
    }
    output
}

fn format_platform(snapshot: &RuntimeSnapshot) -> String {
    let inspection = &snapshot.inspection;
    let csr = &inspection.hart.csrs;
    format!(
        "mstatus {:08x}  mie {:08x}\n\
         mip     {:08x}  mtvec {:08x}\n\
         mepc    {:08x}  mcause {:08x}\n\
         mtval   {:08x}  mscratch {:08x}\n\n\
         mtime {}  mtimecmp {}  pending {}\n\
         PLIC sources {} threshold {} eligible {}\n\
         IRQ routes {}",
        csr.mstatus,
        csr.mie,
        csr.mip,
        csr.mtvec,
        csr.mepc,
        csr.mcause,
        csr.mtval,
        csr.mscratch,
        inspection.timer.time,
        inspection.timer.compare,
        inspection.timer.pending,
        inspection.plic.source_count,
        inspection.plic.threshold,
        inspection.plic.best_eligible_source,
        inspection.irq_route_count,
    )
}

fn format_statistics(snapshot: &RuntimeSnapshot) -> String {
    format!(
        "control status {}\nUART RX dropped {}\nUART TX dropped {}\nUART overrun {}\n\ntranslation {:#?}\nDBT {:#?}",
        snapshot.inspection.control_status,
        snapshot.uart.rx_dropped,
        snapshot.uart.tx_dropped,
        snapshot.uart.receive_overrun,
        snapshot.inspection.translation_stats,
        snapshot.inspection.dbt_stats,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uart_input_remains_visible_until_non_empty_submission() {
        let mut input = UartInputState::default();
        input.text.push_str("Echo me!");

        assert_eq!(input.text, "Echo me!");
        assert_eq!(input.take_submission(), Some(b"Echo me!".to_vec()));
        assert!(input.text.is_empty());
        assert_eq!(input.take_submission(), None);
    }
}
