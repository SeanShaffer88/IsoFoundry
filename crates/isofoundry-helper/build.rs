fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icon.ico");
        res.set("ProductName", "IsoFoundry");
        res.set("FileDescription", "IsoFoundry elevated helper");
        res.set("LegalCopyright", "Copyright (c) 2026 Sean Shaffer. MIT License.");
        // The app starts this helper with the "runas" verb only for tasks that
        // need administrator rights (D-014); the manifest itself stays asInvoker
        // so tests can start it without a UAC prompt.
        res.set_manifest(include_str!("../../assets/helper.manifest"));
        if let Err(e) = res.compile() {
            println!("cargo:warning=Windows resources not embedded: {e}");
        }
    }
}
