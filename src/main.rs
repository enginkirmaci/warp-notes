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
/// [--root] [--note <path>]`. Warp Station's titlebar passes `--root` (the
/// whole store); its repo launchers pass `--repo`. A bare launch roots at
/// ~/notes with nothing pinned. Unknown args are ignored so a stale desktop
/// shortcut never blocks the window.
fn parse_launch() -> Launch {
    let mut launch = Launch::default();
    // args_os: non-UTF-8 argv must degrade to an ignored arg, not panic
    // before the window opens
    let mut args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    while let Some(arg) = args.next() {
        let mut value = || args.next().filter(|v| !v.starts_with('-'));
        match arg.as_str() {
            // resolve_store_root trims and falls back to the shared store, so
            // a stale `--store " "` never becomes a relative directory
            "--store" => {
                launch.store_root = value().map(|v| warp_notes::notes_store::resolve_store_root(&v))
            }
            "--repo" => launch.repo_scope = value(),
            "--root" => launch.whole_store = true,
            "--note" => launch.open_note = value().map(PathBuf::from),
            _ => {}
        }
    }
    launch
}

fn main() {
    // The support contract, before migrations and logging: `--version` answers
    // with the same line the About screen copies.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        let build = warpui_kit_core::build_info!();
        println!(
            "{}",
            build.version_line(warpui_kit_core::about::app(
                warpui_kit_core::about::AppId::Notes
            ))
        );
        return;
    }
    // the data folder moved under ~/.warp and the store to ~/notes: absorb
    // the legacy folders before logging picks its file
    paths::migrate_legacy_layout();
    let _log_guard = logging::initialize();
    // crashes reach the rolling log (the warp-agent services::install_panic_hook pattern)
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default_hook(info);
    }));

    application()
        .with_assets(assets::EmbedAssets::new())
        .run(|cx: &mut App| {
            warpui_kit_core::bootstrap::init(cx, None, false, |cx: &mut gpui::App| {
                theme::apply_kit_theme(cx);
            });
            // the editor's find bar; no key context, so no input swallows it
            cx.bind_keys([gpui::KeyBinding::new(
                "ctrl-f",
                warp_notes::ui::notes::OpenFind,
                None,
            )]);

            let bounds = Bounds::centered(None, size(px(1500.), px(960.)), cx);
            let launch = parse_launch();
            let _ = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_id: Some("warp-notes".into()),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Warp Notes".into()),
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
