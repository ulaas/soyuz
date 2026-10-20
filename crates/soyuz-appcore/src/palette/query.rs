//! Palette query parsing: the first character selects a mode.
//!
//! | prefix | mode |
//! |---|---|
//! | (none) | commands (command palette) / folders (go to folder) |
//! | `:` | go to a path, with folder completion |
//! | `@` | bookmarks, favorites and recent folders |
//! | `/` | recursive search in the current folder |
//! | `>` | run a shell command in the current folder (terminal panel) |
//! | `#` | switch tab |

/// What the palette lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Commands,
    /// Ctrl+P: recent / frequent folders and bookmarks.
    Folders,
    Path,
    Bookmarks,
    Search,
    Shell,
    Tabs,
    /// Ctrl+Alt+V: file sets copied or cut earlier (`[behaviour] clipboard_history`).
    Clipboard,
}

impl Mode {
    /// Prefix character that selects the mode (`None` for the base modes).
    pub fn prefix(self) -> Option<char> {
        match self {
            Mode::Commands | Mode::Folders | Mode::Clipboard => None,
            Mode::Path => Some(':'),
            Mode::Bookmarks => Some('@'),
            Mode::Search => Some('/'),
            Mode::Shell => Some('>'),
            Mode::Tabs => Some('#'),
        }
    }

    /// Chip label shown in the input.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Commands => tr!("actions-mode-commands"),
            Mode::Folders => tr!("actions-mode-folders"),
            Mode::Path => tr!("actions-mode-path"),
            Mode::Bookmarks => tr!("actions-mode-bookmarks"),
            Mode::Search => tr!("actions-mode-search"),
            Mode::Shell => tr!("actions-mode-shell"),
            Mode::Tabs => tr!("actions-mode-tabs"),
            Mode::Clipboard => tr!("actions-mode-clipboard"),
        }
    }

    /// Lowercase name in the footer legend (prefixed modes).
    pub fn legend(self) -> &'static str {
        match self {
            Mode::Commands => tr!("actions-mode-commands"),
            Mode::Folders => tr!("actions-mode-folders"),
            Mode::Path => tr!("actions-legend-path"),
            Mode::Bookmarks => tr!("actions-legend-bookmarks"),
            Mode::Search => tr!("actions-legend-search"),
            Mode::Shell => tr!("actions-legend-shell"),
            Mode::Tabs => tr!("actions-legend-tabs"),
            Mode::Clipboard => tr!("actions-mode-clipboard"),
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Mode::Commands => tr!("actions-placeholder-commands"),
            Mode::Folders => tr!("actions-placeholder-folders"),
            Mode::Path => tr!("actions-placeholder-path"),
            Mode::Bookmarks => tr!("actions-placeholder-bookmarks"),
            Mode::Search => tr!("actions-placeholder-search"),
            Mode::Shell => tr!("actions-placeholder-shell"),
            Mode::Tabs => tr!("actions-placeholder-tabs"),
            Mode::Clipboard => tr!("actions-placeholder-clipboard"),
        }
    }

    /// The prefixed modes, for the footer legend.
    pub const PREFIXED: [Mode; 5] = [Mode::Path, Mode::Bookmarks, Mode::Search, Mode::Shell, Mode::Tabs];
}

/// Split raw input into a mode and the remaining text. `base` is the mode without a
/// prefix (`Commands` or `Folders`). In folder mode an absolute path (`/x`, `~`, `C:\`)
/// is treated as a path rather than as the `/` search prefix.
pub fn parse(input: &str, base: Mode) -> (Mode, &str) {
    let trimmed = input.trim_start();
    if base == Mode::Folders && looks_like_path(trimmed) {
        return (Mode::Path, trimmed);
    }
    let mut chars = trimmed.chars();
    let mode = match chars.next() {
        Some(':') => Mode::Path,
        Some('@') => Mode::Bookmarks,
        Some('/') => Mode::Search,
        Some('>') => Mode::Shell,
        Some('#') => Mode::Tabs,
        _ => return (base, trimmed),
    };
    let rest = chars.as_str();
    // Paths keep their leading spaces stripped; shell commands keep inner spacing.
    (mode, rest.trim_start())
}

/// `C:\`, `C:/`, `\\server`, `/abs`, `~`, `~/x`.
pub fn looks_like_path(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('~')
        || s.starts_with('/')
        || s.starts_with("\\\\")
        || (b.len() >= 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b.len() == 2 || matches!(b[2], b'\\' | b'/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert_eq!(parse("copy", Mode::Commands), (Mode::Commands, "copy"));
        assert_eq!(parse(":C:\\Users", Mode::Commands), (Mode::Path, "C:\\Users"));
        assert_eq!(parse(":  ~/src", Mode::Commands), (Mode::Path, "~/src"));
        assert_eq!(parse("@proj", Mode::Commands), (Mode::Bookmarks, "proj"));
        assert_eq!(parse("/*.rs", Mode::Commands), (Mode::Search, "*.rs"));
        assert_eq!(parse("> git status", Mode::Commands), (Mode::Shell, "git status"));
        assert_eq!(parse("#2", Mode::Commands), (Mode::Tabs, "2"));
        assert_eq!(parse("", Mode::Commands), (Mode::Commands, ""));
        assert_eq!(parse("  new tab", Mode::Commands), (Mode::Commands, "new tab"));
        assert_eq!(parse(">", Mode::Commands), (Mode::Shell, ""));
    }

    #[test]
    fn folder_mode_paths() {
        assert_eq!(parse("/home/me", Mode::Folders), (Mode::Path, "/home/me"));
        assert_eq!(parse("C:\\", Mode::Folders), (Mode::Path, "C:\\"));
        assert_eq!(parse("~", Mode::Folders), (Mode::Path, "~"));
        assert_eq!(parse("proj", Mode::Folders), (Mode::Folders, "proj"));
        assert_eq!(parse("#1", Mode::Folders), (Mode::Tabs, "1"));
        assert!(!looks_like_path("c:x"));
        assert!(looks_like_path("\\\\srv\\share"));
    }

    #[test]
    fn prefix_round_trip() {
        for m in Mode::PREFIXED {
            let c = m.prefix().unwrap();
            assert_eq!(parse(&format!("{c}x"), Mode::Commands).0, m);
        }
    }
}
