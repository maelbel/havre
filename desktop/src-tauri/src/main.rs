// No console window behind the app on Windows (release builds only, so
// `println!` still shows while developing).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    havre_desktop_lib::run();
}
