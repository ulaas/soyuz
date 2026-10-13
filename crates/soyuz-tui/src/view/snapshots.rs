//! Tier-A snapshots (Phase T8 P2): hand-built `ViewModel`s drawn at fixed sizes and
//! compared with `tests/snapshots/view__*.snap` (`crate::snapshot::assert_snapshot`,
//! bless with `SOYUZ_BLESS_SNAPSHOTS=1`). Fully deterministic: no file system, fixed
//! themes (`TuiTheme::plain`, `TuiTheme::from_tokens` with fixed tokens, never
//! `for_core`), en-US.
