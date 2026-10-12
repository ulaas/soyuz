//! The view of the properties dialog (`FrontendCmd::Properties` / `FolderProperties`) — Phase T3.
//!
//! Label / value rows in a scrolling list ([`rows`]), live while the background walk
//! counts a folder or the checksums are computed (a row per algorithm while the checksum
//! section shows, the match ticked; the compare verdict above), then the fields, the
//! buttons and key hints.

use std::time::SystemTime;

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::attrs_edit::{editable_attrs, info_attrs, mode_string};
use soyuz_appcore::models::checksums::{ALGOS, HashState, Verdict, verdict};
use soyuz_appcore::tr;
use soyuz_core::{EntryInfo, format_size, format_time};

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::properties::{State, common_parent};
use crate::theme::Role;
use crate::view::text;
use crate::widgets::ListRow;

/// Preferred width (cells).
pub const WIDTH: u16 = 80;

/// The rows: `(label, value, role of the value)`.
fn pairs(core: &AppCore, s: &State) -> Vec<(String, String, Option<Role>)> {
    let g = &core.config.general;
    let size = |b: u64| format_size(b, g.size_style);
    let exact = |b: u64| if b < 1024 { size(b) } else { tr!("dialogs-props-exact-size", size = size(b), bytes = b) };
    let entries = s.current();
    let single = (entries.len() == 1).then(|| &entries[0]);
    let mut out: Vec<(String, String, Option<Role>)> = Vec::new();
    let mut row = |l: &str, v: String| out.push((l.to_string(), v, None));
    match single {
        Some(e) => {
            row(tr!("tui-ops-name"), safe(&e.name));
            row(tr!("dialogs-props-type"), soyuz_appcore::state::listing::kind_label(e));
        }
        None => row(tr!("tui-ops-name"), tr!("dialogs-props-selected", count = entries.len())),
    }
    if let Some(p) = common_parent(&entries) {
        row(tr!("dialogs-props-location"), safe(&soyuz_fs::strip_verbatim(p).display().to_string()));
    }
    match (&s.stats, single) {
        (None, Some(e)) => {
            row(tr!("dialogs-props-size"), exact(e.size));
            let disk = match e.alloc_size {
                Some(a) => exact(a),
                None => format!("~ {}", size(e.size.div_ceil(4096) * 4096)),
            };
            row(tr!("dialogs-props-size-on-disk"), disk);
        }
        (Some(stats), _) => {
            let st = stats.lock().clone();
            let busy = if st.done { String::new() } else { format!(" {}", tr!("tui-ops-counting")) };
            row(tr!("dialogs-props-size"), format!("{}{busy}", exact(st.bytes)));
            let approx = if st.alloc_known { "" } else { "~ " };
            row(tr!("dialogs-props-size-on-disk"), format!("{approx}{}", exact(st.alloc)));
            let contains = if st.errors > 0 {
                tr!("dialogs-props-contains-errors", files = st.files, dirs = st.dirs, errors = st.errors)
            } else {
                tr!("dialogs-props-contains", files = st.files, dirs = st.dirs)
            };
            let label =
                if entries.len() == 1 { tr!("dialogs-props-contains-label") } else { tr!("dialogs-props-total") };
            row(label, contains);
        }
        (None, None) => {}
    }
    if let Some(t) = s.link.as_ref().and_then(|b| b.with(|t| t.cloned().flatten())) {
        row(tr!("dialogs-props-link-target"), safe(&t));
    }
    let time = |f: fn(&EntryInfo) -> Option<SystemTime>| {
        let first = entries.first().and_then(f);
        if entries.iter().any(|e| f(e) != first) {
            tr!("dialogs-props-multiple").to_string()
        } else {
            first.map(|t| format_time(t, &g.date_format)).unwrap_or_else(|| tr!("dialogs-props-unknown").into())
        }
    };
    row(tr!("dialogs-props-created"), time(|e| e.created));
    row(tr!("dialogs-props-modified"), time(|e| e.modified));
    row(tr!("dialogs-props-accessed"), time(|e| e.accessed));
    let names: Vec<&str> = editable_attrs()
        .into_iter()
        .map(|(a, l, _)| (a, l))
        .chain(info_attrs())
        .filter(|(a, _)| !entries.is_empty() && entries.iter().all(|e| e.attrs.contains(*a)))
        .map(|(_, l)| l)
        .collect();
    row(tr!("dialogs-props-attributes"), if names.is_empty() { "\u{2014}".into() } else { names.join(", ") });
    if let Some(m) = single.and_then(|e| e.mode) {
        row(tr!("dialogs-mode-permissions"), format!("{} ({:o})", mode_string(m), m & 0o7777));
    }
    if let Some(e) = single
        && (e.owner.is_some() || e.group.is_some())
    {
        let who = [e.owner.as_deref(), e.group.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(":");
        row(tr!("dialogs-mode-owner"), safe(&who));
    }
    if let Some(sums) = s.sums.as_ref().filter(|_| s.show_sums) {
        let matched = match verdict(s.compare_text(), &sums.computed()) {
            Verdict::Match(a) => Some(a),
            _ => None,
        };
        for algo in ALGOS {
            let (v, role) = match sums.state(algo) {
                HashState::Idle => (tr!("dialogs-hash-not-computed").to_string(), Some(Role::Faint)),
                st @ HashState::Running { .. } => {
                    let pct = tr!("dialogs-hash-percent", percent = format!("{:.0}", st.fraction() * 100.0));
                    (format!("{} {pct}", tr!("tui-ops-hashing")), Some(Role::Muted))
                }
                HashState::Done(h) if matched == Some(algo) => (format!("{h} \u{2713}"), Some(Role::Success)),
                HashState::Done(h) => (h, None),
                HashState::Failed(e) => (safe(&e), Some(Role::Danger)),
            };
            out.push((algo.label().to_string(), v, role));
        }
    }
    out
}

/// The rows of the list, labels padded to one column.
pub fn rows(core: &AppCore, s: &State) -> Vec<ListRow> {
    let p = pairs(core, s);
    let w = p.iter().map(|(l, _, _)| text::width(l)).max().unwrap_or(0);
    p.into_iter()
        .map(|(l, v, role)| {
            let pad = " ".repeat(w.saturating_sub(text::width(&l)));
            ListRow { text: format!("{l}{pad}  {v}"), right: String::new(), role }
        })
        .collect()
}

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = WIDTH;
    d.list = rows(core, s);
    d.list_state = s.list.clone();
    d.hints = if s.show_sums { tr!("tui-ops-sums-hints") } else { tr!("tui-ops-props-hints") }.to_string();
    if s.show_sums
        && let Some(sums) = &s.sums
        && let Some(msg) = verdict(s.compare_text(), &sums.computed()).message()
    {
        d.body = vec![format!("{}: {msg}", tr!("dialogs-hash-compare"))];
    }
    d
}
