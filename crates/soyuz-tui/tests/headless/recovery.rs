//! Headless tests of T3: interrupted jobs and crash reports. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
