//! The About section: the family in the list, the selected member in the
//! detail. The screen itself is `warpui_kit_core::about`; this holds the one
//! piece of state it needs and the effects behind its verbs, so every app's
//! About behaves the same and only the rail plumbing differs.

use crate::ui::notes::{NotesPage, NoticeKind};
use gpui::prelude::*;
use gpui::{App, ClipboardItem, Context, Window};
use warpui_kit_core::shell;

/// The license text ships with the binary, not only with the repository.
pub(crate) const LICENSE: &str = include_str!("../../LICENSE");

impl NotesPage {
    /// The rail's About section: the four suite members in the list, the
    /// selected one's identity, Build and Legal cards in the detail. It opens
    /// on Notes, the app you are in.
    pub(crate) fn render_about(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        use warpui_kit_core::about::{self, AppId};

        let build = warpui_kit_core::build_info!();
        let width = self.prefs.list_width;
        let selected = self.about_selected;
        let current = AppId::Notes;

        let entity = cx.entity();
        let pick = entity.clone();
        let on_pick = move |id: AppId, _: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            pick.update(cx, |this, cx| {
                this.about_selected = id;
                cx.notify();
            });
        };
        let act = entity.clone();
        let on_action =
            move |action: about::Action, _: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
                act.update(cx, |this, cx| this.apply_about_action(action, cx));
            };
        let open = entity;
        let on_link =
            move |link: about::Link, _: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
                open.update(cx, |this, cx| this.open_about_link(link, cx));
            };

        shell::panes()
            .min_w_0()
            .debug_selector(|| "about-page".into())
            .child(about::list(width, selected, current, &build, on_pick))
            .child(about::detail(
                selected,
                current,
                &build,
                "Open Station",
                on_action,
                on_link,
            ))
            .into_any_element()
    }

    /// What the About detail's buttons do. Notes can launch its siblings but
    /// cannot reach Station's Tools page, so SnapIt's "Open controls" here
    /// means "open Station".
    pub(crate) fn apply_about_action(
        &mut self,
        action: warpui_kit_core::about::Action,
        cx: &mut Context<Self>,
    ) {
        use warpui_kit_core::about;

        match action {
            about::Action::ReleaseNotes => {
                let url = format!("{}/releases", about::repository());
                cx.open_url(&url);
            }
            about::Action::OpenApp(id) => {
                self.launch_sibling(about::app(id).binary, cx);
            }
            about::Action::SnapIt => self.launch_sibling("warp-station", cx),
            about::Action::CopyVersion => {
                let build = warpui_kit_core::build_info!();
                let line = build.version_line(about::app(self.about_selected));
                cx.write_to_clipboard(ClipboardItem::new_string(line));
            }
        }
    }

    /// The three external links open the browser; the license stays in the app,
    /// where the text ships with the binary.
    pub(crate) fn open_about_link(
        &mut self,
        link: warpui_kit_core::about::Link,
        cx: &mut Context<Self>,
    ) {
        use warpui_kit_core::about;

        if link == about::Link::License {
            self.about_license_open = true;
            cx.notify();
            return;
        }
        let url = link.url(about::app(self.about_selected));
        cx.open_url(&url);
    }

    fn launch_sibling(&mut self, binary: &'static str, cx: &mut Context<Self>) {
        if let Err(err) = warpui_kit_platform::sibling::open(binary) {
            self.notice(cx, NoticeKind::Error, &err);
        }
    }
}
