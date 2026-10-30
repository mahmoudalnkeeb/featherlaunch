//! Embeds the app icon in the Windows executable (Explorer, taskbar pins, shortcuts).

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    if let Err(e) = res.compile() {
        // MSVC builds (CI, releases) must have the icon. The GNU toolchain is only used for
        // `cargo check` here and may lack windres, so it just warns.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            panic!("embedding the icon failed: {e}");
        }
        println!("cargo:warning=icon not embedded: {e}");
    }
}
