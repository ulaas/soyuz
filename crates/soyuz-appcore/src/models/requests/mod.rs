//! The dialogs the core requests from the frontend (`FrontendCmd`), without drawing:
//! confirm, select by pattern, columns, open with, interrupted-job recovery and crash
//! reports. The GUI (`soyuz-app/src/ui/dialogs`) and the terminal UI
//! (`soyuz-tui/src/input/dialogs`) draw them and call these rules, so both behave the same.

pub mod columns;
pub mod confirm;
pub mod crash_reports;
pub mod open_with;
pub mod pattern;
pub mod recovery;
