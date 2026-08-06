// Hide the console window on Windows release builds. Debug builds keep it so
// `tracing` output stays visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dev_control_center_lib::run()
}
