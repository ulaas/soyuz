use super::*;

fn entries() -> Vec<(Action, catalog::Entry)> {
    Action::all().into_iter().map(|a| (a.clone(), catalog::entry(&a))).collect()
}

fn top(q: &str, recent: &[Action]) -> Vec<Action> {
    let e = entries();
    score_actions(&e, recent, q).into_iter().take(5).map(|r| e[r.index].0.clone()).collect()
}

#[test]
fn ranks_obvious_commands_first() {
    assert_eq!(top("new folder", &[])[0], Action::NewFolder);
    assert_eq!(top("settings", &[])[0], Action::Settings);
    assert_eq!(top("copy path", &[])[0], Action::CopyPath);
    assert_eq!(top("dual", &[])[0], Action::ToggleDualPane);
    // Keywords and descriptions match too ("recycle" is not in any title).
    assert!(top("recycle", &[]).contains(&Action::Delete));
}

#[test]
fn recent_commands_get_boosted() {
    let plain = top("tab", &[]);
    let boosted = top("tab", &[Action::DuplicateTab]);
    assert_ne!(plain[0], Action::DuplicateTab);
    assert_eq!(boosted[0], Action::DuplicateTab);
    let e = entries();
    let r = score_actions(&e, &[Action::DuplicateTab], "tab");
    assert!(r.iter().any(|m| m.recent && e[m.index].0 == Action::DuplicateTab));
}

#[test]
fn highlights_follow_title_matches() {
    let e = entries();
    let r = score_actions(&e, &[], "nf");
    let first = &r[0];
    assert!(!first.hl.is_empty());
    let title: Vec<char> = e[first.index].1.title.chars().collect();
    for i in &first.hl {
        assert!(title[*i as usize].eq_ignore_ascii_case(&'n') || title[*i as usize].eq_ignore_ascii_case(&'f'));
    }
}

#[test]
fn no_match_is_empty() {
    let e = entries();
    assert!(score_actions(&e, &[], "zzqqxx").is_empty());
    // Empty query matches every action.
    assert_eq!(score_actions(&e, &[], "").len(), e.len());
}

/// Every action can run from the palette: it has palette metadata, is offered as an
/// item, and running the item dispatches exactly that action through `actions::execute`.
#[test]
fn every_action_is_runnable_from_the_palette() {
    let e = entries();
    let all = score_actions(&e, &[], "");
    assert_eq!(all.len(), Action::all().len());
    for (a, entry) in &e {
        assert!(!entry.title.is_empty() && !entry.icon.is_empty() && !entry.description.is_empty(), "{a}");
        // The item runs the action itself (no remapping).
        let item = Item::new(Exec::Action(a.clone()), entry.title.clone(), entry.icon, "Commands");
        assert_eq!(item.exec, Exec::Action(a.clone()));
        // String form round-trips, so recent commands persist.
        assert_eq!(a.to_string().parse::<Action>().unwrap(), *a);
    }
}

#[test]
fn completion_texts() {
    let it = Item::new(Exec::Shell("git status".into()), "x", ph::TERMINAL, "s");
    assert_eq!(completion_of(&it).as_deref(), Some(">git status"));
    let it = Item::new(Exec::Action(Action::Copy), "x", ph::COPY, "s");
    assert_eq!(completion_of(&it), None);
    let dir = std::env::temp_dir();
    let it = Item::new(Exec::Go(Location::Dir(dir.clone())), "x", ph::FOLDER, "s");
    let text = completion_of(&it).unwrap();
    assert!(text.starts_with(':') && text.ends_with(std::path::MAIN_SEPARATOR));
}

#[test]
fn tilde_expansion() {
    let home = soyuz_fs::home();
    assert_eq!(PathBuf::from(expand_tilde("~")), home);
    assert_eq!(PathBuf::from(expand_tilde("~/x")), home.join("x"));
    assert_eq!(expand_tilde("/abs"), "/abs");
}
