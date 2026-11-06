//! The `?` overlay: the keys of the page, and the filter language on pages with a process
//! filter. It scrolls with the arrows.

use soyuz_appcore::models::monitor::labels;
use soyuz_appcore::tr;
use soyuz_core::config::monitor::MonitorPage;

use super::Cx;
use super::canvas::Area;
use crate::theme::Role;

/// The page's own help text, one entry per line (`key  what`).
fn page_text(page: MonitorPage) -> &'static str {
    match page {
        MonitorPage::Overview => tr!("tui-monitor-help-overview"),
        MonitorPage::Processes => tr!("tui-monitor-help-processes"),
        MonitorPage::Cpu => tr!("tui-monitor-help-cpu"),
        MonitorPage::Memory | MonitorPage::Disks | MonitorPage::Gpu | MonitorPage::Sensors => {
            tr!("tui-monitor-help-devices")
        }
        MonitorPage::Network => tr!("tui-monitor-help-network"),
        MonitorPage::Connections => tr!("tui-monitor-help-connections"),
        MonitorPage::Services => tr!("tui-monitor-help-services"),
        MonitorPage::Startup => tr!("tui-monitor-help-lists"),
        MonitorPage::History => tr!("tui-monitor-help-history"),
        MonitorPage::Snapshots => tr!("tui-monitor-help-snapshots"),
    }
}

/// The overlay's lines; a heading has `true`.
pub(super) fn lines(page: MonitorPage) -> Vec<(String, bool)> {
    let mut out = vec![(labels::page_title(page).to_string(), true)];
    out.extend(page_text(page).lines().map(|l| (l.trim().to_string(), false)));
    out.push((String::new(), false));
    out.push((tr!("tui-monitor-help-all-title").to_string(), true));
    out.extend(tr!("tui-monitor-help-all").lines().map(|l| (l.trim().to_string(), false)));
    if page == MonitorPage::Processes {
        out.push((String::new(), false));
        out.push((tr!("tui-monitor-help-query-title").to_string(), true));
        out.extend(tr!("tui-monitor-help-query").lines().map(|l| (l.trim().to_string(), false)));
    }
    out
}

pub(super) fn draw(cx: &mut Cx<'_>, a: Area, page: MonitorPage) {
    let all = lines(page);
    let w = a.w.min(100);
    let h = a.h.min(all.len() as u16 + 3);
    let x = a.x + (a.w - w) / 2;
    let y = a.y + i32::from((a.h - h) / 2);
    let area = Area { x, y, w, h };
    cx.c.blank(area);
    if w == 0 || h == 0 {
        return;
    }
    let ascii = cx.look.ascii || cx.look.sr;
    let inner = if h >= 3 && w >= 4 && !cx.look.sr {
        let (tl, tr_, bl, br, hz, vt) =
            if ascii { ("+", "+", "+", "+", "-", "|") } else { ("┌", "┐", "└", "┘", "─", "│") };
        let line = hz.repeat(usize::from(w - 2));
        cx.c.text(x, y, w, &format!("{tl}{line}{tr_}"), Role::Border);
        cx.c.text(x, y + i32::from(h) - 1, w, &format!("{bl}{line}{br}"), Role::Border);
        for r in 1..h - 1 {
            cx.c.text(x, y + i32::from(r), 1, vt, Role::Border);
            cx.c.text(x + w - 1, y + i32::from(r), 1, vt, Role::Border);
        }
        Area { x: x + 2, y: y + 1, w: w - 4, h: h - 2 }
    } else {
        area
    };
    let scroll = cx.nav.help.unwrap_or(0).min(all.len().saturating_sub(usize::from(inner.h)));
    cx.nav.help = Some(scroll);
    for (i, (s, head)) in all.iter().skip(scroll).take(usize::from(inner.h)).enumerate() {
        let row = inner.y + i as i32;
        if *head {
            cx.c.bold(inner.x, row, inner.w, s, Role::Title);
        } else {
            cx.c.text(inner.x, row, inner.w, s, Role::Text);
        }
    }
    cx.c.set_caret(inner.x, inner.y);
    cx.announce = tr!("tui-monitor-help-title").to_string();
}
