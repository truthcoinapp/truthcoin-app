// Prevents an extra console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    truthcoin_app::keep_inherited_files_from_children();
    truthcoin_app::run()
}
