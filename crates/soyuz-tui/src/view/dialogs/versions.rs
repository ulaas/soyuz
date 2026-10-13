//! The view of the previous versions dialog (`TuiDialog::Versions`, agent B): where the
//! versions come from, then the state of the scan (looking, an error, why nothing was
//! found, how many copies need administrator rights — a message only), the versions
//! (snapshot time, size and modified time), the compare of the selected version with the
//! current file (after C), the buttons and key hints. A note about denied copies draws
//! the box with the warning border.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::versions::{
    Comparison, denied_more_text, elevate_hint, empty_text, intro_text, looking_text, snapshot_label, version_detail,
};
use soyuz_appcore::tr;
use soyuz_core::config::Config;
use soyuz_core::{format_size, format_time};

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::versions::State;
use crate::widgets::ListRow;

/// Preferred width (cells).
pub const WIDTH: u16 = 76;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    view(&core.config, s)
}

fn view(config: &Config, s: &State) -> DialogVm {
    let g = &config.general;
    let m = &s.model;
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = WIDTH;
    d.body = vec![intro_text().to_string()];
    d.hints = if m.is_dir { tr!("tui-versions-hints-folder") } else { tr!("tui-versions-hints") }.to_string();
    let report = match &m.report {
        None => {
            d.body.push(looking_text().to_string());
            return d;
        }
        Some(Err(e)) => {
            d.body.push(format!("{}: {}", tr!("tools-versions-list-failed"), safe(e)));
            return d;
        }
        Some(Ok(r)) => r,
    };
    if let Some(u) = &report.unsupported {
        d.body.push(format!("{}: {}", tr!("tools-versions-unsupported"), safe(u)));
        return d;
    }
    let hint = elevate_hint(report, m.is_dir);
    d.alert = hint.is_some();
    if report.versions.is_empty() {
        // Denied copies: how to get them (never a button); else why there are none.
        let (title, body) = hint.unwrap_or_else(|| empty_text(report, m.is_dir));
        d.body.push(title);
        d.body.push(body);
        return d;
    }
    if let Some((_, body)) = hint {
        d.body.push(denied_more_text(report.denied));
        d.body.push(body);
    }
    d.list = report
        .versions
        .iter()
        .map(|v| ListRow {
            text: safe(&snapshot_label(v, &g.date_format)),
            right: version_detail(v, &g.date_format, g.size_style),
            role: None,
        })
        .collect();
    d.list_state = s.list.clone();
    if let Some(c) = &m.compare
        && let Some(v) = report.versions.get(c.index)
    {
        let side = |label: &str, size: Option<u64>, modified: Option<std::time::SystemTime>| {
            let size = size.map_or_else(|| tr!("tools-versions-deleted").to_string(), |b| format_size(b, g.size_style));
            match modified {
                Some(t) => format!(
                    "{label}: {size}  ·  {}",
                    tr!("tools-versions-modified", date = format_time(t, &g.date_format))
                ),
                None => format!("{label}: {size}"),
            }
        };
        let cur = m.current;
        d.text = vec![
            side(tr!("tools-versions-current"), cur.map(|x| x.0), cur.and_then(|x| x.1)),
            side(tr!("tools-versions-this"), Some(v.size), v.modified),
            Comparison::text(c.result.as_ref()),
        ];
    }
    d
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_appcore::models::versions::VersionsModel;
    use soyuz_fs::snapshots::{PathVersion, Snapshot, VersionsReport};

    use super::*;
    use crate::layout::Hits;
    use crate::theme::TuiTheme;
    use crate::view::dialogs::draw_dialog;

    fn report(n: u32, denied: usize) -> VersionsReport {
        let versions = (1..=n)
            .map(|seq| PathVersion {
                snapshot: Snapshot {
                    provider: "Shadow copy",
                    id: format!("s{seq}"),
                    root: PathBuf::from(format!("snap{seq}")),
                    volume: PathBuf::from("vol"),
                    created: None,
                    seq,
                },
                path: PathBuf::from(format!("snap{seq}")).join("a.txt"),
                size: 2048,
                modified: None,
                is_dir: false,
            })
            .collect();
        VersionsReport { versions, denied, ..VersionsReport::default() }
    }

    fn state(r: Result<VersionsReport, String>) -> State {
        State::new(VersionsModel::with_report(PathBuf::from("report.docx"), false, r))
    }

    #[test]
    fn lists_versions_with_their_details() {
        let d = view(&Config::default(), &state(Ok(report(2, 0))));
        assert!(d.title.contains("report.docx"), "{}", d.title);
        assert_eq!(d.list.len(), 2);
        assert_eq!(d.list[1].text, "Shadow copy #2");
        assert!(d.list[0].right.contains("KiB"));
        assert!(!d.alert);
        assert!(d.text.is_empty(), "no compare yet");
    }

    #[test]
    fn denied_copies_are_a_message_without_a_button() {
        let d = view(&Config::default(), &state(Ok(report(0, 2))));
        assert!(d.alert);
        assert!(d.list.is_empty());
        let (title, body) = elevate_hint(&report(0, 2), false).unwrap();
        assert_eq!(&d.body[1..], [title, body]);
        assert!(d.buttons.iter().all(|b| b.label != tr!("tools-versions-elevate")));
        // With versions: the count of the others above the list, and the same hint.
        let d = view(&Config::default(), &state(Ok(report(1, 3))));
        assert_eq!(d.body[1], denied_more_text(3));
        assert_eq!(d.list.len(), 1);
        // Nothing denied, nothing found: why.
        let d = view(&Config::default(), &state(Ok(report(0, 0))));
        assert_eq!(d.body[1], empty_text(&report(0, 0), false).0);
        let d = view(&Config::default(), &state(Err("boom".into())));
        assert!(d.body[1].ends_with("boom"));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let mut long = report(12, 4);
        long.versions[0].modified = Some(std::time::SystemTime::UNIX_EPOCH);
        for s in [state(Ok(long)), state(Ok(report(0, 2))), state(Err("x".into()))] {
            let d = view(&Config::default(), &s);
            for w in 0..=40 {
                for h in 0..=12 {
                    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                    let mut hits = Hits::default();
                    t.draw(|f| draw_dialog(f, f.area(), &d, &TuiTheme::plain(), &mut hits)).unwrap();
                }
            }
        }
    }
}
