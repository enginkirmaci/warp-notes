fn main() {
    // Windows only: embed the app icon so Explorer, shortcuts and the taskbar
    // show the mark. The winresource build-dep is windows-gated, so this block
    // must vanish on every other host.
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=packaging/windows/warp-notes.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/warp-notes.ico");
        if let Err(err) = res.compile() {
            panic!("could not embed Windows resources: {err}");
        }
    }
}
