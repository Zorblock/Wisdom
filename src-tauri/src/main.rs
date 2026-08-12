#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Rust 1.97's `linker_messages` filter only recognizes link.exe's English
// "Creating library" progress line. A localized MSVC toolchain reports the
// same harmless line as a warning, so suppress it for the Windows executable.
#![cfg_attr(target_env = "msvc", allow(linker_messages))]

fn main() {
    wisdom_lib::run();
}
