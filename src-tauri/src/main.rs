// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `mouthpiece …` from a terminal or an agent runs the same binary; it
    // talks to the running app and exits without opening a window.
    if let Some(code) = tauri_app_lib::cli::run_if_cli() {
        std::process::exit(code);
    }
    tauri_app_lib::run()
}
