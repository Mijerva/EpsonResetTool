#![windows_subsystem = "windows"]

fn main() {
    #[cfg(windows)]
    epson_reset::gui::run();
}
