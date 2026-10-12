//! Headless tests of T3: the clipboard (OSC 52, files). Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
