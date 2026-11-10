/// The About screen's build facts. The release scripts (`scripts/build.sh`,
/// `scripts/build.ps1`) export these three; a plain `cargo build` without a
/// script still compiles and reports `dev`, never a lie about being a release.
fn build_info() {
    let var = |key: &str, fallback: &str| {
        println!("cargo:rerun-if-env-changed={key}");
        std::env::var(key).unwrap_or_else(|_| fallback.to_string())
    };
    println!(
        "cargo:rustc-env=WARP_SUITE_BUILD={}",
        var("WARP_SUITE_BUILD", "dev")
    );
    println!(
        "cargo:rustc-env=WARP_SUITE_CHANNEL={}",
        var("WARP_SUITE_CHANNEL", "dev")
    );
    println!(
        "cargo:rustc-env=WARP_SUITE_RELEASED={}",
        var("WARP_SUITE_RELEASED", "unknown")
    );
}

fn main() {
    build_info();
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
