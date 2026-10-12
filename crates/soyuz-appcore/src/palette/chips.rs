//! Shortcut chips as text: how a binding (`"Ctrl+Shift+P"`, `"G G"`) splits into keys and
//! how each key is shown. Frontends draw the chips; this is the shared, toolkit-free part.

/// Display form of one key name (`"Up"` → `"↑"`).
pub fn key_glyph(k: &str) -> String {
    match k {
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Enter" => "↵".into(),
        "Backspace" => "⌫".into(),
        "Tab" => "⇥".into(),
        "PageUp" => "PgUp".into(),
        "PageDown" => "PgDn".into(),
        "Cmd" if cfg!(target_os = "macos") => "⌘".into(),
        "Alt" if cfg!(target_os = "macos") => "⌥".into(),
        "Shift" if cfg!(target_os = "macos") => "⇧".into(),
        "Ctrl" if cfg!(target_os = "macos") => "⌃".into(),
        other => other.to_string(),
    }
}

/// Split `"Ctrl+Shift+P"` / `"G G"` / `"Ctrl++"` into key groups (one group per chord of
/// a sequence, one entry per key of a chord).
pub fn split_keys(binding: &str) -> Vec<Vec<String>> {
    binding
        .split_whitespace()
        .map(|chord| {
            let mut keys: Vec<String> = Vec::new();
            let (head, plus) = match chord.strip_suffix("++") {
                Some(h) => (h, true),
                None if chord == "+" => ("", true),
                None => (chord, false),
            };
            if !head.is_empty() {
                keys.extend(head.split('+').filter(|s| !s.is_empty()).map(key_glyph));
            }
            if plus {
                keys.push("+".into());
            }
            keys
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_splitting() {
        assert_eq!(split_keys("Ctrl+Shift+P"), vec![vec!["Ctrl", "Shift", "P"]]);
        assert_eq!(split_keys("G G"), vec![vec!["G"], vec!["G"]]);
        assert_eq!(split_keys("Ctrl++"), vec![vec!["Ctrl", "+"]]);
        assert_eq!(split_keys("Alt+Up"), vec![vec!["Alt".to_string(), "↑".to_string()]]);
        assert!(split_keys("").is_empty());
    }
}
