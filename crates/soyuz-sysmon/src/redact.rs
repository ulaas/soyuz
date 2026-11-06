//! Masks secrets in command lines before they leave the process in a snapshot or an export.
//! This is a best effort over free-form text: it prefers masking too much, and `SnapshotCommands`
//! lets the user leave command lines out altogether.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use soyuz_core::config::monitor::SnapshotCommands;

use crate::frame::ProcRow;

const MASK: &str = "***";

static SECRET_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(pass|pwd|token|secret|key|auth|cred)").expect("valid regex"));
static URL_CREDENTIALS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z][A-Za-z0-9+.\-]*://[^/\s:@]*:)[^/\s@]+@").expect("valid regex"));

static URL_QUERY_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([?&][^=&\s]*(?:pass|pwd|token|secret|key|auth|cred)[^=&\s]*=)[^&\s]+").expect("valid regex")
});

/// Splits on whitespace outside quotes. A `"` quotes anywhere; a `'` only at the start of a
/// word or a value, since apostrophes occur in paths. An unfinished quote runs to the end.
fn tokens(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut prev = ' ';
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c.is_whitespace() => {
                if let Some(st) = start.take() {
                    out.push((st, i));
                }
            }
            None => {
                if start.is_none() {
                    start = Some(i);
                }
                if c == '"' || (c == '\'' && (prev.is_whitespace() || prev == '=' || prev == ':')) {
                    quote = Some(c);
                }
            }
        }
        prev = c;
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

/// A word that names a secret on its own: `--password`, `-Token`, `/key` (not a path).
fn is_secret_flag(word: &str) -> bool {
    let dashed = word.starts_with('-');
    let slashed = word.starts_with('/') && !word[1..].contains(['/', '\\']);
    (dashed || slashed) && SECRET_NAME.is_match(word)
}

/// `-p value`, as many database and login tools take a password.
fn is_short_password_flag(word: &str) -> bool {
    word == "-p"
}

/// `-psecret`: the value glued to the flag.
fn glued_password(word: &str) -> Option<usize> {
    (word.len() > 2 && word.starts_with("-p") && !word.starts_with("--")).then_some(2)
}

/// `name=value` or `name:value` with a name that looks like a secret: where the value starts.
fn secret_value_at(word: &str) -> Option<usize> {
    let sep = word.find(['=', ':'])?;
    if word[sep + 1..].starts_with("//") {
        return None; // a URL; its credentials are handled below
    }
    SECRET_NAME.is_match(&word[..sep]).then_some(sep + 1)
}

/// Masks the values of flags that look like secrets and the password in `scheme://user:pass@host`.
pub fn redact_cmdline(cmd: &str) -> Cow<'_, str> {
    let mut out = String::with_capacity(cmd.len());
    let mut copied = 0;
    let mut mask_next = false;
    let mut changed = false;
    for (start, end) in tokens(cmd) {
        let word = &cmd[start..end];
        out.push_str(&cmd[copied..start]);
        copied = end;

        let masked_now = std::mem::take(&mut mask_next);
        let looks_like_scheme =
            masked_now && matches!(word.to_ascii_lowercase().as_str(), "bearer" | "basic" | "token");
        let flag = is_secret_flag(word) || is_short_password_flag(word);
        let value_at = secret_value_at(word);
        let empty_value = value_at.is_some_and(|at| at == word.len());

        if flag && value_at.is_none() {
            // A masked word can itself be a flag: `--key-file --password x` must not leak x.
            mask_next = true;
            if masked_now {
                out.push_str(MASK);
                changed = true;
            } else {
                out.push_str(word);
            }
        } else if masked_now {
            out.push_str(MASK);
            changed = true;
            mask_next = looks_like_scheme;
        } else if let Some(at) = value_at {
            out.push_str(&word[..at]);
            if empty_value {
                mask_next = true; // `Authorization: Bearer abc`, split by the OS
            } else {
                out.push_str(MASK);
                changed = true;
            }
        } else if let Some(at) = glued_password(word) {
            out.push_str(&word[..at]);
            out.push_str(MASK);
            changed = true;
        } else {
            out.push_str(word);
        }
    }
    out.push_str(&cmd[copied..]);

    if let Cow::Owned(clean) = URL_CREDENTIALS.replace_all(&out, format!("${{1}}{MASK}@")) {
        out = clean;
        changed = true;
    }
    if let Cow::Owned(clean) = URL_QUERY_SECRET.replace_all(&out, format!("${{1}}{MASK}")) {
        out = clean;
        changed = true;
    }
    if changed { Cow::Owned(out) } else { Cow::Borrowed(cmd) }
}

/// A copy of a row with its command line as the setting asks.
pub fn apply_to_row(row: &ProcRow, mode: SnapshotCommands) -> ProcRow {
    let mut row = row.clone();
    match mode {
        SnapshotCommands::Full => {}
        SnapshotCommands::None => row.raw.cmdline = None,
        SnapshotCommands::Redacted => {
            if let Some(c) = &row.raw.cmdline
                && let Cow::Owned(masked) = redact_cmdline(c)
            {
                row.raw.cmdline = Some(Arc::from(masked));
            }
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        redact_cmdline(s).into_owned()
    }

    #[test]
    fn ordinary_commands_stay_untouched_and_borrowed() {
        for cmd in [
            "firefox.exe --new-tab https://example.org:8080/path",
            r"C:\Program Files\App\app.exe /quiet /log:C:\x.log",
            "/usr/bin/bash -c 'ls -la'",
            "cargo build --release",
            "",
        ] {
            assert!(matches!(redact_cmdline(cmd), Cow::Borrowed(_)), "{cmd}");
        }
    }

    #[test]
    fn values_after_secret_flags_are_masked() {
        assert_eq!(r("app --password hunter2 --verbose"), "app --password *** --verbose");
        assert_eq!(r("app --api-key abc"), "app --api-key ***");
        assert_eq!(r("app --TOKEN=abc next"), "app --TOKEN=*** next");
        assert_eq!(r("app --client-secret:abc"), "app --client-secret:***");
        assert_eq!(r("app /pwd abc"), "app /pwd ***");
        assert_eq!(r("sshpass -p hunter2 ssh host"), "sshpass -p *** ssh host");
        assert_eq!(r("mysql -phunter2 db"), "mysql -p*** db");
    }

    #[test]
    fn name_value_words_are_masked() {
        assert_eq!(
            r("env PASSWORD=abc AWS_SECRET_ACCESS_KEY=def run"),
            "env PASSWORD=*** AWS_SECRET_ACCESS_KEY=*** run"
        );
        assert_eq!(r("curl https://x/?api_key=abc&y=1"), "curl https://x/?api_key=***&y=1");
    }

    #[test]
    fn quoted_values_are_masked_whole() {
        assert_eq!(r(r#"app --password "a b c" --next"#), "app --password *** --next");
        assert_eq!(r("app --password 'a b c' --next"), "app --password *** --next");
        assert_eq!(r(r#"curl -H "Authorization: Bearer abc" x"#), "curl -H \"Authorization:*** x");
        assert_eq!(r(r#"app --password "unfinished secret"#), "app --password ***");
    }

    #[test]
    fn a_header_split_by_the_os_is_masked_through_its_scheme() {
        assert_eq!(r("curl -H Authorization: Bearer abc x"), "curl -H Authorization: *** *** x");
    }

    #[test]
    fn a_flag_in_the_value_position_does_not_hide_the_next_secret() {
        assert_eq!(r("app --key-file --password hunter2"), "app --key-file *** ***");
    }

    #[test]
    fn url_credentials_are_masked() {
        assert_eq!(r("git clone https://bob:hunter2@host/repo"), "git clone https://bob:***@host/repo");
        assert_eq!(r("psql postgres://u:p%40ss@db:5432/x"), "psql postgres://u:***@db:5432/x");
        assert_eq!(r("psql postgres://:p@db"), "psql postgres://:***@db");
        assert_eq!(r("ftp ftp://anon@host"), "ftp ftp://anon@host");
    }

    #[test]
    fn apostrophes_in_paths_do_not_swallow_the_rest() {
        assert_eq!(r("/home/bob's/app --token abc --x"), "/home/bob's/app --token *** --x");
    }

    #[test]
    fn whitespace_between_words_is_kept() {
        assert_eq!(r("a   --key  v   b"), "a   --key  ***   b");
    }

    #[test]
    fn modes() {
        let mut row = ProcRow::default();
        row.raw.cmdline = Some(Arc::from("app --password x"));
        assert_eq!(apply_to_row(&row, SnapshotCommands::None).raw.cmdline, None);
        assert_eq!(apply_to_row(&row, SnapshotCommands::Full).raw.cmdline.as_deref(), Some("app --password x"));
        assert_eq!(apply_to_row(&row, SnapshotCommands::Redacted).raw.cmdline.as_deref(), Some("app --password ***"));
    }
}
