//! Windows: put the app icon (assets/icon/ironswift.ico, every size from 16 to 256) and the
//! product name into ironswift.exe, so Explorer, shortcuts and the taskbar show the iron bird
//! instead of the generic program icon. The window's title bar icon is set at run time
//! (window.rs). Other platforms take their icons from packaging (packaging/).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/ironswift.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/ironswift.ico");
    res.set("ProductName", "Ironswift");
    res.set("FileDescription", "Ironswift web browser");
    res.set("CompanyName", "Ironswift");
    // Without the Windows resource compiler (Visual Studio's or MinGW's) the program still
    // builds, just with the generic icon.
    if let Err(e) = res.compile() {
        println!("cargo:warning=the Windows app icon wasn't embedded: {e}");
    }
}
