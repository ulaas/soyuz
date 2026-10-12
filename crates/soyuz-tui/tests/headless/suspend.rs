//! Headless tests of T5 / T6: suspend to a shell or a text editor (W1-suspend). Each test runs in
//! its own sandbox: `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
