fn main() {
    println!("cargo:rerun-if-changed=rust/assets/app.ico");
    println!("cargo:rerun-if-changed=rust/assets/app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("rust/assets/app.ico")
            .set_manifest_file("rust/assets/app.manifest")
            .set("ProductName", "ThodShot")
            .set("FileDescription", "ThodShot")
            .compile()
            .expect("Windows resources");
    }
}
