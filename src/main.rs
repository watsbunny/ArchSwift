//! ArchSwift — a graphical wizard for installing local Arch Linux packages.
//!
//! The program is meant to be started by double-clicking a `*.pkg.tar.zst` file.
//! The architecture is deliberately split into three layers:
//!   1. `metadata`  — safe reading of `.PKGINFO` from a compressed tar archive;
//!   2. `installer` — runs `pkexec pacman -U` in a separate thread, without blocking GTK;
//!   3. `ui`        — a three-step GTK4 wizard exchanging messages over a channel.
//!
//! Every file-system and external-process operation returns a clear localized
//! error. User data is never passed to a shell: `pkexec` is invoked directly
//! through `std::process::Command` with an argument array.

use std::path::PathBuf;

use gtk::glib;
use gtk::prelude::*;
use gtk::{gio, Application};

/// Single logging entry point. The prefix makes searching journalctl/stdout easier.
/// The macro is declared before the modules so `log_info!` is visible in every child module.
macro_rules! log_info {
    ($($arg:tt)*) => {
        eprintln!("[{}] {}", crate::APP_NAME, format_args!($($arg)*))
    };
}

mod i18n;
mod installer;
mod metadata;
mod ui;

/// Application identifier in freedesktop.org / Wayland / D-Bus terms.
pub const APP_ID: &str = "org.openarch.swift";
/// Human-readable name used in window titles and logs.
pub const APP_NAME: &str = "ArchSwift";
/// Icon name of the application itself from the hicolor theme.
pub const ICON_APP: &str = "arch-swift-installer";

/// Messages that background threads send to the GTK thread.
///
/// The channel intentionally carries no GTK types: that keeps it usable from
/// plain `std::thread`s without breaking GTK's thread-safety rules.
enum WorkerMsg {
    /// Parsed package metadata. The box keeps channel messages small.
    Metadata(Result<Box<metadata::PackageInfo>, String>),
    /// A single line of `pkexec`/`pacman` output.
    Log(String),
    /// Installation outcome.
    Finished(installer::InstallOutcome),
}

fn main() -> glib::ExitCode {
    // Icons are packed into a GResource at build time, so the wizard looks the
    // same both before the package is installed and after it.
    gio::resources_register_include!("arch-swift.gresource")
        .expect("failed to register the bundled resources");

    // The `%f` argument from the .desktop file: the path of the double-clicked package.
    // When started manually without arguments, the wizard offers a file chooser.
    let initial_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());

    // NON_UNIQUE allows several wizards to run at once — one per package.
    // This is closer to the usual behaviour of Windows installers.
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |application| {
        log_info!(
            "Starting {} (application-id: {}, language: {})",
            APP_NAME,
            APP_ID,
            i18n::language()
        );
        ui::build(application, initial_path.clone());
    });

    // GApplication must not treat the package path as a file argument: without the
    // HANDLES_OPEN flag that produces a critical "This application can not open
    // files" message and an error exit. The path has already been read manually
    // above, so GTK only receives the program name.
    let program = std::env::args()
        .next()
        .unwrap_or_else(|| APP_NAME.to_string());

    app.run_with_args(&[program])
}
