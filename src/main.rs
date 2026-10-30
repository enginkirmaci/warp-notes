//! WarpNotes — per-repo markdown notes with an Obsidian-style live editor.
// no console window behind the packaged Windows exe; debug builds keep it for logs
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use warp_notes::ui::notes::{Launch, NotesPage};
use warp_notes::{assets, logging, paths, theme};

use gpui::prelude::*;
use gpui::{App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_platform::application;
use std::path::PathBuf;

/// The launch contract: `warp-notes [--store <path>] [--repo <name>]
/// [--note <path>]`. DevTools passes all three; a bare launch roots at
/// ~/.warp/notes with nothing pinned. Unknown args are ignored so a stale
/// desktop shortcut never blocks the window.
fn parse_launch() -> Launch {
    let mut launch = Launch::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().filter(|v| !v.starts_with('-'));
        match arg.as_str() {
            "--store" => launch.store_root = value().map(PathBuf::from),
            "--repo" => launch.repo_scope = value(),
            "--note" => launch.open_note = value().map(PathBuf::from),
            _ => {}
        }
    }
    launch
}

fn main() {
    let _log_guard = logging::initialize();
    // the shared store moves out of ~/.devtools once, on whoever runs first
    paths::migrate_legacy_store();

    application()
        .with_assets(assets::EmbedAssets::new())
        .run(|cx: &mut App| {
            // kit components (input decorations, popovers) + our palette mapped
            // onto their theme tokens — must run before any kit component renders
            gpui_kit::init(cx);
            theme::apply_kit_theme(cx);

            let bounds = Bounds::centered(None, size(px(980.), px(700.)), cx);
            let launch = parse_launch();
            let _ = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_id: Some("warp-notes".into()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Notes".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| NotesPage::new(launch, window, cx)),
            );
            cx.activate(true);
        });
}
