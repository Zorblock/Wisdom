fn main() {
    slint_build::compile("ui/app.slint").expect("Unable to compile Slint UI");
    println!("cargo:rerun-if-changed=assets/wisdom.ico");
    println!("cargo:rerun-if-changed=package.json");
    if cfg!(target_os = "windows") {
        let package: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string("package.json").expect("Unable to read package.json"),
        )
        .expect("package.json is not valid JSON");
        let version = package["version"]
            .as_str()
            .expect("package.json must contain a version string");

        let mut resources = winres::WindowsResource::new();
        resources.set_icon("assets/wisdom.ico");
        resources.set("CompanyName", "zorblock");
        resources.set("FileDescription", "Wisdom");
        resources.set("ProductName", "Wisdom");
        resources.set("FileVersion", version);
        resources.set("ProductVersion", version);
        resources.set("OriginalFilename", "wisdom.exe");
        resources.compile().expect("Unable to embed Windows application icon");
    }
}
