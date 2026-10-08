//! Key-binding conflict analysis for the settings editor: duplicate chords within a
//! context, sequences that can never fire (a prefix is bound on its own), and global
//! bindings shadowed by a more specific context.

use soyuz_core::config::{Action, KeyChord, KeyContext, KeySequence, Keymap, format_sequence};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConflictKind {
    /// Two actions share a chord in one context: only one of them can ever run.
    Duplicate,
    /// A sequence starts with a chord (or shorter sequence) that is bound on its own.
    Prefix,
    /// A global chord is taken by another action in a specific context.
    Shadowed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub context: KeyContext,
    /// The binding as written (`"Ctrl+K"`, `"G G"`).
    pub keys: String,
    pub actions: Vec<Action>,
}

impl Conflict {
    pub fn message(&self) -> String {
        let names: Vec<String> = self.actions.iter().map(crate::palette::catalog::title).collect();
        let context = crate::i18n::labels::key_context(self.context);
        match self.kind {
            ConflictKind::Duplicate => {
                let actions = names
                    .into_iter()
                    .reduce(|first, second| tr!("actions-conflict-and", first = first, second = second))
                    .unwrap_or_default();
                tr!("actions-conflict-duplicate", keys = &self.keys, actions = actions, context = context)
            }
            ConflictKind::Prefix => {
                tr!("actions-conflict-prefix", keys = &self.keys, actions = names.join(" / "))
            }
            ConflictKind::Shadowed => tr!(
                "actions-conflict-shadowed",
                keys = &self.keys,
                other = names.get(1).cloned().unwrap_or_default(),
                context = soyuz_core::locale::to_lower_in(context, crate::i18n::current()),
                action = names.first().cloned().unwrap_or_default()
            ),
        }
    }
}

/// Every conflict in `km`, sorted (duplicates first).
pub fn analyze(km: &Keymap) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = km
        .conflicts_with_context()
        .into_iter()
        .map(|(context, chord, actions)| Conflict {
            kind: ConflictKind::Duplicate,
            context,
            keys: chord.to_string(),
            actions,
        })
        .collect();
    for (context, short, long, actions) in km.prefix_conflicts() {
        out.push(Conflict {
            kind: ConflictKind::Prefix,
            context,
            keys: format!("{} / {}", format_sequence(&short), format_sequence(&long)),
            actions,
        });
    }
    for ((ctx, action), chords) in &km.bindings {
        if *ctx != KeyContext::Global {
            continue;
        }
        for c in chords {
            for ((other_ctx, other), others) in &km.bindings {
                if *other_ctx == KeyContext::Global || other == action || !others.contains(c) {
                    continue;
                }
                // Text fields and dialogs never see most global chords anyway.
                if matches!(other_ctx, KeyContext::Omnibar | KeyContext::Dialog | KeyContext::Rename) {
                    continue;
                }
                out.push(Conflict {
                    kind: ConflictKind::Shadowed,
                    context: *other_ctx,
                    keys: c.to_string(),
                    actions: vec![action.clone(), other.clone()],
                });
            }
        }
    }
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.keys.cmp(&b.keys)));
    out
}

/// Conflicts that involve `action`.
pub fn for_action<'a>(all: &'a [Conflict], action: &Action) -> Vec<&'a Conflict> {
    all.iter().filter(|c| c.actions.contains(action)).collect()
}

/// Which actions currently use `seq` (any context), for the recorder's warning.
pub fn users_of(km: &Keymap, seq: &[KeyChord]) -> Vec<(KeyContext, Action)> {
    let mut v = Vec::new();
    if seq.len() == 1 {
        for ((ctx, a), chords) in &km.bindings {
            if chords.contains(&seq[0]) {
                v.push((*ctx, a.clone()));
            }
        }
    } else {
        for ((ctx, a), seqs) in &km.sequences {
            if seqs.iter().any(|s: &KeySequence| s.as_slice() == seq) {
                v.push((*ctx, a.clone()));
            }
        }
    }
    v
}

/// Extension used above (kept local to the app; core's `conflicts` drops the context).
trait ConflictsWithContext {
    fn conflicts_with_context(&self) -> Vec<(KeyContext, KeyChord, Vec<Action>)>;
}

impl ConflictsWithContext for Keymap {
    fn conflicts_with_context(&self) -> Vec<(KeyContext, KeyChord, Vec<Action>)> {
        let mut map: std::collections::BTreeMap<(KeyContext, KeyChord), Vec<Action>> = Default::default();
        for ((ctx, action), chords) in &self.bindings {
            for c in chords {
                map.entry((*ctx, *c)).or_default().push(action.clone());
            }
        }
        map.into_iter().filter(|(_, v)| v.len() > 1).map(|((ctx, c), v)| (ctx, c, v)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{Config, KeyPreset, defaults, keymap, parse_sequence};

    fn ch(s: &str) -> KeyChord {
        s.parse().unwrap()
    }

    #[test]
    fn presets_are_clean_of_hard_conflicts() {
        for p in KeyPreset::ALL {
            let km = defaults::preset(p);
            let hard: Vec<_> = analyze(&km).into_iter().filter(|c| c.kind != ConflictKind::Shadowed).collect();
            assert!(hard.is_empty(), "{p:?}: {hard:?}");
        }
    }

    #[test]
    fn detects_each_kind() {
        let mut km = Keymap::default();
        km.bind(KeyContext::Global, Action::Refresh, ch("F5"));
        km.bind(KeyContext::FileList, Action::CopyToOtherPane, ch("F5"));
        km.bind(KeyContext::FileList, Action::Copy, ch("Ctrl+C"));
        km.bind(KeyContext::FileList, Action::CopyPath, ch("Ctrl+C"));
        km.bind(KeyContext::FileList, Action::Rename, ch("C"));
        km.bind_sequence(KeyContext::FileList, Action::NewFolder, parse_sequence("c w").unwrap());
        let all = analyze(&km);
        let kinds: Vec<_> = all.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, vec![ConflictKind::Duplicate, ConflictKind::Prefix, ConflictKind::Shadowed]);
        assert!(all[0].message().contains("Ctrl+C"));
        assert!(all[2].message().contains("file list"), "{}", all[2].message());
        assert_eq!(for_action(&all, &Action::Refresh).len(), 1);
        assert_eq!(users_of(&km, &[ch("F5")]).len(), 2);
        assert_eq!(users_of(&km, &parse_sequence("c w").unwrap()), vec![(KeyContext::FileList, Action::NewFolder)]);
    }

    #[test]
    fn overrides_steal_instead_of_conflicting() {
        let mut c = Config::default();
        c.keys.bindings.insert("refresh".into(), vec!["Ctrl+T".into()]);
        let (km, w) = keymap(&c);
        assert!(w.is_empty());
        let dup: Vec<_> = analyze(&km).into_iter().filter(|c| c.kind == ConflictKind::Duplicate).collect();
        assert!(dup.is_empty(), "{dup:?}");
        assert_eq!(km.lookup(KeyContext::Global, ch("Ctrl+T")), Some(&Action::Refresh));
    }
}
