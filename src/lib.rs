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

pub mod profile;
pub mod runtime;
pub mod terminal;
pub mod view_model;

#[cfg(test)]
mod tests {
    use crate::profile::MachineProfile;
    use crate::runtime::RuntimeHandle;
    use crate::terminal::TerminalProjection;
    use crate::view_model::PlaygroundViewModel;

    fn assert_send<T: Send>() {}

    #[test]
    fn application_boundaries_are_independently_owned() {
        assert_send::<MachineProfile>();
        assert_send::<RuntimeHandle>();
        assert_send::<TerminalProjection>();
        assert_send::<PlaygroundViewModel>();
    }
}
