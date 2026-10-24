fn main() {
    // Embed the app icon as Windows resource ID 1, but only when actually
    // building on a Windows host (winresource needs a Windows resource
    // compiler). Cross-checks from other hosts simply skip it; the tray
    // code falls back to a stock icon if the resource is missing.
    let target_is_windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    if target_is_windows && cfg!(windows) {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "TrayClick");
        res.set("FileDescription", "TrayClick — tray auto clicker");
        if let Err(e) = res.compile() {
            println!("cargo:warning=failed to embed icon resource: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
