//! Headless tests of T5: the embedded terminal pane (W2-terminal-pane). Each test runs in its own
//! sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
