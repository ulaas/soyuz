//! Embeds the app icon and version info into the Windows executable `soyuz-tui.exe`
//! (the same `icon.ico` as `soyuz.exe`, `crates/soyuz-app/assets/icon.ico`). Version
//! numbers come from the crate version (`winresource` fills `FileVersion` /
//! `ProductVersion`); the git commit is `soyuz_appcore::GIT_SHA`.
//!
//! Needs the Windows SDK resource compiler (`rc.exe`, present with the MSVC build tools).
//! When it is missing the build continues without the resource. No-op on other targets.

fn main() {
    const ICON: &str = "../soyuz-app/assets/icon.ico";
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=build.rs");
    let target_windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    // MSVC only: there `winresource` passes the `.res` as a link argument of this
    // package's own binaries. With the GNU toolchain it links a static library instead,
    // which would also reach `soyuz.exe` (it depends on this crate) and clash with
    // `soyuz-app`'s own resource.
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !(cfg!(windows) && target_windows && msvc) {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ICON)
        .set("ProductName", "Soyuz")
        .set("FileDescription", "Soyuz terminal UI")
        .set("InternalName", "soyuz-tui")
        .set("OriginalFilename", "soyuz-tui.exe")
        .set("LegalCopyright", "Copyright (c) 2026 Aydemir Şahin and the Soyuz contributors. MIT License.");
    if let Err(e) = res.compile() {
        println!("cargo:warning=soyuz-tui: icon not embedded ({e}); install the Windows SDK for rc.exe");
    }
}
