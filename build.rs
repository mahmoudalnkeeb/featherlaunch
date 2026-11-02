//! Embeds the app icon, version details and manifest in the Windows executable. Explorer and the
//! taskbar show the icon and details, and the manifest states that the launcher needs no admin rights.

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "FeatherLaunch")
        .set("FileDescription", "FeatherLaunch Minecraft launcher")
        .set("CompanyName", "Mahmoud Alnakeeb")
        .set("LegalCopyright", "Copyright (C) 2026 Mahmoud Alnakeeb")
        .set("InternalName", "featherlaunch")
        .set("OriginalFilename", "featherlaunch.exe")
        .set_manifest(MANIFEST);
    if let Err(e) = res.compile() {
        // MSVC builds (CI, releases) must have the icon. The GNU toolchain is only used for
        // `cargo check` here and may lack windres, so it just warns.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            panic!("embedding the icon failed: {e}");
        }
        println!("cargo:warning=icon not embedded: {e}");
    }
}
