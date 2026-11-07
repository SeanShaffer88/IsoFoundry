fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=../../assets/app.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        // Resource id 1: window/taskbar icon and the footer logo.
        res.set_icon_with_id("../../assets/icon.ico", "1");
        res.set("ProductName", "IsoFoundry");
        res.set("FileDescription", "IsoFoundry");
        res.set("LegalCopyright", "Copyright (c) 2026 Sean Shaffer. MIT License.");
        res.set_manifest(include_str!("../../assets/app.manifest"));
        if let Err(e) = res.compile() {
            println!("cargo:warning=Windows resources not embedded: {e}");
        }
    }
}
