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

use blinc_app::prelude::*;
use blinc_app::windowed::WindowedApp;

pub fn run() -> Result<()> {
    WindowedApp::run(WindowConfig::default(), |ctx| {
        div()
            .w(ctx.width)
            .h(ctx.height)
            .items_center()
            .justify_center()
            .child(text("Compukter Playground").size(28.0))
    })
}
