//! The app: a WinUI 3 window running [`light_note::win::Shell`].
//!
//! Everything interesting is in the library — the core (`geom`, `ink`, `shape`,
//! `doc`, `otd`, `pdf`) and the host (`win::shell`) — so `main` has one job: hand
//! the host component to `windows-reactor` and let it own the window and the
//! message loop.  No setup, no globals, no `unsafe` here.
//!
//! A release build has no console window; a debug build keeps it, because that is
//! where `windows-reactor` and the workers report their problems.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    if let Err(error) = light_note::win::shell::run() {
        eprintln!("light-note could not start: {error}");
    }

    #[cfg(not(windows))]
    eprintln!("light-note is a Windows 11 app: the drawing pad, the PDF engine and the UI all come from Win32/WinUI 3");
}