use std::env;

// Embed an application manifest into the binary. The manifest declares a
// dependency on Common-Controls v6, which native-windows-gui needs for the
// comctl32 window-subclassing API to resolve at load time.
//
// `/MANIFEST:EMBED` uses the MSVC linker's built-in manifest embedding, so no
// separate resource compiler (rc.exe) is required.
fn main() {
    let manifest = format!("{}/app.manifest", env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{manifest}");

    // Compile the application icon (icon.rc -> RT_GROUP_ICON id 1) into the
    // binary, so both the exe's file icon and the tray icon come from it.
    println!("cargo:rerun-if-changed=icon.rc");
    println!("cargo:rerun-if-changed=icon.ico");
    let _ = embed_resource::compile("icon.rc", embed_resource::NONE);
}
