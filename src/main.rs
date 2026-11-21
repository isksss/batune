// Release builds on Windows are GUI-subsystem apps so no console window opens
// beside the terminal. Debug builds keep the console for `eprintln!`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    batune::run();
}
