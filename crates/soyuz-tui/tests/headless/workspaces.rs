//! Headless tests of Phase T7 (agent A): workspaces (save as, manage, hotkeys), tab
//! history, the close-window path and About. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`, like the other
//! files here. Skeleton (wave 0): agent A adds the tests.
