//! Headless tests of T3: copy, move, trash, new items and links through the jobs engine. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`.
