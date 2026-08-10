fn main() {
    slint_build::compile("ui/app.slint").expect("Unable to compile Slint UI");
    println!("cargo:rerun-if-changed=assets/wisdom.ico");
    if cfg!(target_os = "windows") {
        let mut resources = winres::WindowsResource::new();
        resources.set_icon("assets/wisdom.ico");
        resources.compile().expect("Unable to embed Windows application icon");
    }
}
