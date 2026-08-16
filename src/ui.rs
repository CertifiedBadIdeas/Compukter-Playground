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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use blinc_app::prelude::*;
use blinc_app::windowed::{WindowedApp, WindowedContext};

use compukter_playground::profile::RuntimeMode;
use compukter_playground::profile_catalog::ProfileEntry;
use compukter_playground::runtime::{RuntimeCommand, RuntimeSnapshot};
use compukter_playground::terminal::TerminalMode;
use compukter_playground::view_model::PlaygroundViewModel;

const BACKGROUND: Color = Color::rgba(0.035, 0.047, 0.067, 1.0);
const PANEL: Color = Color::rgba(0.075, 0.094, 0.125, 1.0);
const BORDER: Color = Color::rgba(0.18, 0.22, 0.29, 1.0);
const TEXT: Color = Color::rgba(0.88, 0.91, 0.96, 1.0);
const MUTED: Color = Color::rgba(0.55, 0.62, 0.72, 1.0);
const BUTTON_BACKGROUND: Color = Color::rgba(0.090, 0.196, 0.302, 1.0);
const BUTTON_BORDER: Color = Color::rgba(0.192, 0.341, 0.475, 1.0);
const BUTTON_HOVER_BACKGROUND: Color = Color::rgba(0.137, 0.278, 0.400, 1.0);
const BUTTON_HOVER_BORDER: Color = Color::rgba(0.255, 0.443, 0.608, 1.0);

type SharedViewModel = Arc<Mutex<PlaygroundViewModel>>;

pub fn run() -> Result<()> {
    let view_model = Arc::new(Mutex::new(PlaygroundViewModel::from_profiles_dir(
        "profiles",
    )));
    let uart_input = text_input_state_with_placeholder("UART input");
    let poller_started = Arc::new(AtomicBool::new(false));
    let poller_running = Arc::new(AtomicBool::new(true));
    let config = WindowConfig {
        title: "Compukter Playground".to_string(),
        width: 1440,
        height: 900,
        min_size: Some((960, 640)),
        ..WindowConfig::default()
    };

    let running_for_ui = Arc::clone(&poller_running);
    let result = WindowedApp::run(config, move |ctx| {
        let refresh = ctx.use_state_keyed("runtime-refresh", || 0_u64);
        let _ = refresh.get();
        if !poller_started.swap(true, Ordering::SeqCst) {
            let running = Arc::clone(&running_for_ui);
            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(50));
                    refresh.update_rebuild(|revision| revision.wrapping_add(1));
                }
            });
        }

        workbench(ctx, &view_model, &uart_input)
    });
    poller_running.store(false, Ordering::SeqCst);
    result
}

fn workbench(
    ctx: &mut WindowedContext,
    view_model: &SharedViewModel,
    uart_input: &SharedTextInputState,
) -> Div {
    let snapshot = view_model.lock().unwrap().snapshot();

    div()
        .w(ctx.width)
        .h(ctx.height)
        .bg(BACKGROUND)
        .flex_col()
        .gap(2.0)
        .p(3.0)
        .child(toolbar(ctx, view_model, snapshot.as_deref()))
        .child(
            div()
                .flex_row()
                .flex_1()
                .gap(3.0)
                .overflow_clip()
                .child(terminal_panel(
                    ctx,
                    view_model,
                    uart_input,
                    snapshot.as_deref(),
                ))
                .child(inspector_panel(snapshot.as_deref())),
        )
        .child(status_bar(view_model, snapshot.as_deref()))
}

fn toolbar(
    ctx: &WindowedContext,
    view_model: &SharedViewModel,
    snapshot: Option<&RuntimeSnapshot>,
) -> Div {
    let paused = snapshot.is_none_or(|snapshot| snapshot.paused);
    let mode = snapshot.map(|snapshot| snapshot.mode);

    div()
        .w_full()
        .flex_row()
        .gap(2.0)
        .items_center()
        .child(text("Compukter Playground").size(20.0).color(TEXT))
        .child(profile_selector(ctx, view_model))
        .child(action_button(ctx, "save", "Save", {
            let view_model = Arc::clone(view_model);
            move || {
                if let Err(error) = view_model.lock().unwrap().save_profile() {
                    view_model.lock().unwrap().set_status_error(error);
                }
            }
        }))
        .child(action_button(
            ctx,
            "pause",
            if paused { "Run" } else { "Pause" },
            command_action(Arc::clone(view_model), RuntimeCommand::SetPaused(!paused)),
        ))
        .child(action_button(
            ctx,
            "step",
            "Step",
            command_action(Arc::clone(view_model), RuntimeCommand::Step),
        ))
        .child(action_button(
            ctx,
            "reset",
            "Reset",
            command_action(Arc::clone(view_model), RuntimeCommand::Reset),
        ))
        .child(action_button(
            ctx,
            "mode",
            match mode {
                Some(RuntimeMode::Unbounded) => "Unbounded",
                _ => "Realtime 20 TPS",
            },
            command_action(
                Arc::clone(view_model),
                RuntimeCommand::SetMode(match mode {
                    Some(RuntimeMode::Unbounded) => RuntimeMode::Realtime,
                    _ => RuntimeMode::Unbounded,
                }),
            ),
        ))
}

fn profile_selector(ctx: &WindowedContext, view_model: &SharedViewModel) -> Div {
    let view_model_guard = view_model.lock().unwrap();
    let active_name = view_model_guard
        .profile_path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Select profile".to_string());
    let entries = view_model_guard.profiles().to_vec();
    drop(view_model_guard);

    let hovered = ctx.use_state_keyed("profile-selector", || false);
    let (background, border) = button_colors(hovered.get());
    let selector = div()
        .px(3.0)
        .py(1.5)
        .rounded(6.0)
        .bg(background)
        .border(1.0, border)
        .items_center()
        .justify_center()
        .child(
            text(format!("Profile: {active_name}"))
                .size(12.0)
                .color(TEXT)
                .no_cursor(),
        );

    if entries.is_empty() {
        return selector;
    }

    let overlay_manager = ctx.overlay_manager();
    let view_model = Arc::clone(view_model);
    selector
        .cursor_pointer()
        .on_hover_enter({
            let hovered = hovered.clone();
            move |_| hovered.set(true)
        })
        .on_hover_leave(move |_| hovered.set(false))
        .on_click(move |event| {
            let entries = entries.clone();
            let view_model = Arc::clone(&view_model);
            let manager_for_content = Arc::clone(&overlay_manager);
            let width = event.bounds_width.max(180.0);
            let height = entries.len() as f32 * 28.0 + 4.0;
            overlay_manager
                .dropdown()
                .at(event.bounds_x, event.bounds_y + event.bounds_height)
                .size(width, height)
                .dismiss_on_escape(true)
                .dismiss_on_click_outside(true)
                .animation(OverlayAnimation::none())
                .content(move || {
                    profile_dropdown(
                        &entries,
                        Arc::clone(&view_model),
                        Arc::clone(&manager_for_content),
                        width,
                    )
                })
                .show();
        })
}

fn profile_dropdown(
    entries: &[ProfileEntry],
    view_model: SharedViewModel,
    overlay_manager: OverlayManager,
    width: f32,
) -> Div {
    let mut menu = div()
        .w(width)
        .flex_col()
        .gap(1.0)
        .p(1.0)
        .rounded(6.0)
        .bg(PANEL)
        .border(1.0, BORDER);

    for entry in entries {
        let name = entry.name().to_owned();
        let state_key = format!("profile-entry:{name}");
        let view_model = Arc::clone(&view_model);
        let overlay_manager = Arc::clone(&overlay_manager);
        menu = menu.child(
            Stateful::with_shared_state(use_shared_state::<ButtonState>(&state_key))
                .w_full()
                .h(27.0)
                .px(3.0)
                .items_center()
                .rounded(5.0)
                .cursor_pointer()
                .child(text(name.clone()).size(12.0).color(TEXT).no_cursor())
                .on_state(|state, item| {
                    let hovered = matches!(state, ButtonState::Hovered | ButtonState::Pressed);
                    let (background, border) = button_colors(hovered);
                    *item = item.swap().bg(background).border(1.0, border);
                })
                .on_click(move |_| {
                    let result = view_model.lock().unwrap().select_profile(&name);
                    if let Err(error) = result {
                        view_model.lock().unwrap().set_status_error(error);
                    }
                    overlay_manager.close_top();
                }),
        );
    }
    menu
}

fn terminal_panel(
    ctx: &WindowedContext,
    view_model: &SharedViewModel,
    uart_input: &SharedTextInputState,
    snapshot: Option<&RuntimeSnapshot>,
) -> Div {
    let terminal_mode = snapshot
        .map(|snapshot| snapshot.terminal.mode)
        .unwrap_or(TerminalMode::Ansi);
    let output = snapshot
        .map(|snapshot| match terminal_mode {
            TerminalMode::Ansi => snapshot.terminal.ansi_text.clone(),
            TerminalMode::Raw => format_raw(&snapshot.terminal.raw_bytes),
        })
        .unwrap_or_else(|| "Open a TOML machine profile to start the VM.".to_string());
    let connected = snapshot.is_some_and(|snapshot| snapshot.uart_connected);

    div()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(2.0)
        .p(2.0)
        .bg(PANEL)
        .border(1.0, BORDER)
        .rounded(8.0)
        .overflow_clip()
        .child(
            div()
                .flex_row()
                .gap(2.0)
                .child(text("UART terminal").size(16.0).color(TEXT))
                .child(action_button(
                    ctx,
                    "terminal-mode",
                    match terminal_mode {
                        TerminalMode::Ansi => "ANSI / VT100",
                        TerminalMode::Raw => "Raw bytes",
                    },
                    command_action(
                        Arc::clone(view_model),
                        RuntimeCommand::SetTerminalMode(match terminal_mode {
                            TerminalMode::Ansi => TerminalMode::Raw,
                            TerminalMode::Raw => TerminalMode::Ansi,
                        }),
                    ),
                ))
                .child(action_button(
                    ctx,
                    "uart-connect",
                    if connected { "Disconnect" } else { "Connect" },
                    command_action(
                        Arc::clone(view_model),
                        RuntimeCommand::SetUartConnected(!connected),
                    ),
                ))
                .child(action_button(
                    ctx,
                    "terminal-clear",
                    "Clear",
                    command_action(Arc::clone(view_model), RuntimeCommand::ClearTerminal),
                )),
        )
        .child(
            div()
                .flex_1()
                .w_full()
                .p(2.0)
                .bg(Color::rgba(0.015, 0.021, 0.031, 1.0))
                .overflow_scroll()
                .child(text(output).size(13.0).color(TEXT).monospace()),
        )
        .child(
            div()
                .w_full()
                .flex_row()
                .gap(2.0)
                .child(text_input(uart_input).w_full().text_size(13.0))
                .child(action_button(ctx, "uart-send", "Send", {
                    let view_model = Arc::clone(view_model);
                    let uart_input = Arc::clone(uart_input);
                    move || {
                        let bytes = {
                            let mut input = uart_input.lock().unwrap();
                            let bytes = input.value.as_bytes().to_vec();
                            input.value.clear();
                            input.cursor = 0;
                            bytes
                        };
                        if !bytes.is_empty() {
                            let mut view_model = view_model.lock().unwrap();
                            if let Err(error) = view_model.command(RuntimeCommand::SendUart(bytes))
                            {
                                view_model.set_status_error(error);
                            }
                        }
                    }
                })),
        )
}

fn inspector_panel(snapshot: Option<&RuntimeSnapshot>) -> Div {
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

    div()
        .w(440.0)
        .h_full()
        .flex_col()
        .gap(2.0)
        .overflow_scroll()
        .child(info_card("Registers", registers))
        .child(info_card("CSR / timer / PLIC", platform))
        .child(info_card("Execution", stats))
}

fn info_card(title: &str, contents: String) -> Div {
    div()
        .w_full()
        .flex_col()
        .gap(1.0)
        .p(2.0)
        .bg(PANEL)
        .border(1.0, BORDER)
        .rounded(8.0)
        .child(text(title).size(15.0).color(TEXT))
        .child(text(contents).size(12.0).color(MUTED).monospace())
}

fn status_bar(view_model: &SharedViewModel, snapshot: Option<&RuntimeSnapshot>) -> Div {
    let view_model = view_model.lock().unwrap();
    let status = snapshot.map_or_else(
        || "Stopped".to_string(),
        |snapshot| {
            format!(
                "{:?} | {:?} | revision {}",
                snapshot.outcome, snapshot.mode, snapshot.revision
            )
        },
    );
    let message = snapshot
        .and_then(|snapshot| snapshot.error.as_deref())
        .or_else(|| view_model.status_message())
        .unwrap_or("");
    div()
        .w_full()
        .flex_row()
        .gap(3.0)
        .child(text(status).size(12.0).color(MUTED))
        .child(text(message).size(12.0).color(TEXT))
}

fn action_button(
    ctx: &WindowedContext,
    key: &'static str,
    label: impl Into<String>,
    action: impl Fn() + Send + Sync + 'static,
) -> Div {
    let hovered = ctx.use_state_keyed(key, || false);
    let (background, border) = button_colors(hovered.get());

    div()
        .px(3.0)
        .py(1.5)
        .rounded(6.0)
        .bg(background)
        .border(1.0, border)
        .cursor_pointer()
        .items_center()
        .justify_center()
        .child(text(label).size(12.0).color(TEXT).no_cursor())
        .on_hover_enter({
            let hovered = hovered.clone();
            move |_| hovered.set(true)
        })
        .on_hover_leave(move |_| hovered.set(false))
        .on_click(move |_| action())
}

fn button_colors(hovered: bool) -> (Color, Color) {
    if hovered {
        (BUTTON_HOVER_BACKGROUND, BUTTON_HOVER_BORDER)
    } else {
        (BUTTON_BACKGROUND, BUTTON_BORDER)
    }
}

fn command_action(view_model: SharedViewModel, command: RuntimeCommand) -> impl Fn() + Send + Sync {
    move || {
        let mut view_model = view_model.lock().unwrap();
        if let Err(error) = view_model.command(command.clone()) {
            view_model.set_status_error(error);
        }
    }
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
    fn button_colors_become_lighter_when_hovered() {
        let (idle_background, idle_border) = button_colors(false);
        let (hover_background, hover_border) = button_colors(true);

        assert_eq!(idle_background, BUTTON_BACKGROUND);
        assert_eq!(idle_border, BUTTON_BORDER);
        assert!(hover_background.r > idle_background.r);
        assert!(hover_background.g > idle_background.g);
        assert!(hover_background.b > idle_background.b);
        assert!(hover_border.r > idle_border.r);
        assert!(hover_border.g > idle_border.g);
        assert!(hover_border.b > idle_border.b);
    }
}
