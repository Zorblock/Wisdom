fn main() {
    println!("cargo:rerun-if-changed=../assets/wisdom.ico");
    tauri_build::build()
}
