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

use std::fs;

use compukter_playground::profile_catalog::{ProfileCatalog, ProfileCatalogError};
use tempfile::tempdir;

#[test]
fn catalog_contains_only_sorted_direct_toml_files() {
    let temporary = tempdir().unwrap();
    let profiles = temporary.path().join("profiles");
    fs::create_dir(&profiles).unwrap();
    fs::write(profiles.join("zeta.toml"), "").unwrap();
    fs::write(profiles.join("alpha.toml"), "").unwrap();
    fs::write(profiles.join("notes.txt"), "").unwrap();
    fs::create_dir(profiles.join("nested")).unwrap();
    fs::write(profiles.join("nested/hidden.toml"), "").unwrap();

    let catalog = ProfileCatalog::scan(&profiles).unwrap();
    let names: Vec<_> = catalog.entries().iter().map(|entry| entry.name()).collect();

    assert_eq!(names, ["alpha.toml", "zeta.toml"]);
}

#[test]
fn default_profile_is_preferred_over_alphabetical_first() {
    let temporary = tempdir().unwrap();
    let profiles = temporary.path().join("profiles");
    fs::create_dir(&profiles).unwrap();
    fs::write(profiles.join("alpha.toml"), "").unwrap();
    fs::write(profiles.join("default.toml"), "").unwrap();

    let catalog = ProfileCatalog::scan(&profiles).unwrap();

    assert_eq!(catalog.startup_entry().name(), "default.toml");
}

#[test]
fn empty_profile_directory_is_rejected() {
    let temporary = tempdir().unwrap();
    let profiles = temporary.path().join("profiles");
    fs::create_dir(&profiles).unwrap();

    let error = ProfileCatalog::scan(&profiles).unwrap_err();

    assert!(matches!(error, ProfileCatalogError::Empty { path } if path == profiles));
}
