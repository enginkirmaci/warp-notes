//! Build facts and the Windows resource block come from the shared kit; the
//! icon path is relative to this app's manifest directory.

fn main() {
    warpui_kit_build::emit(
        warpui_kit_build::AppInfo::new(
            "Warp Notes",
            "Markdown notes for repositories",
            "warp-notes.exe",
        )
        .with_icon("packaging/windows/warp-notes.ico"),
    );
}
