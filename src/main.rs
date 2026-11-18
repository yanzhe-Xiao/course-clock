#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod import;
mod model;
mod sync;
mod ui;
mod win;

fn main() {
    if let Err(error) = app::run() {
        win::alert(std::ptr::null_mut(), "课时钟", &error);
    }
}
