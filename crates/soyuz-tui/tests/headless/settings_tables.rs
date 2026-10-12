//! Headless tests of T6: arrays / maps of tables and secrets in the settings editor (W3 settings
//! tables). Each test runs in its own sandbox: `crate::common::relaunch(module_path!(), "<test
//! name>", |data| ...)`.
