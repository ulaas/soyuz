//! Headless tests of Phase T7 (agent B): device rows and menus in the sidebar, unlocking,
//! the read-only offer. Each test runs in its own sandbox:
//! `crate::common::relaunch(module_path!(), "<test name>", |data| ...)`, like the other
//! files here.
//!
//! UDisks2 never runs: the core's test seam (`models::devices::set_desk_backend`) gives
//! the devices and answers the unlock and the mount that follows it, on every platform.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyModifiers};
use soyuz_appcore::models::devices::{self, DeskBackend};
use soyuz_core::config::{Config, KeyContext, SidebarSection, UnmountedDevices};
use soyuz_fs::udisks::{BlockDevice, CandidateState, MountOptions};
use soyuz_tui::app::App;
use soyuz_tui::view::sidebar::LineKind;
use soyuz_tui::widgets::MenuItem;
use soyuz_tui::widgets::form::FieldVmKind;

use crate::common::{self, Run, Step, alt, ctrl, dialog_title, key, key_mods};

const OBJ: &str = "/org/freedesktop/UDisks2/block_devices/";

/// What the fake backend was asked, in order ("unlock sdb2 <secret>", "mount dm0").
type Calls = Arc<Mutex<Vec<String>>>;

/// UDisks2 stand-in: an NTFS stick partition and a locked BitLocker one; "right" (or the
/// recovery key below) unlocks, anything else is a wrong passphrase.
struct Fake {
    calls: Calls,
    mount_at: PathBuf,
}

/// A recovery key as the backend gets it (normalized by the core).
const KEY: &str = "111111-222222-333333-444444-555555-666666-777777-888888";

fn short(object: &str) -> &str {
    object.strip_prefix(OBJ).unwrap_or(object)
}

impl DeskBackend for Fake {
    fn available(&self) -> bool {
        true
    }
    fn block_devices(&self) -> std::io::Result<Vec<BlockDevice>> {
        let stick = |name: &str| BlockDevice {
            object: format!("{OBJ}{name}"),
            device: PathBuf::from(format!("/dev/{name}")),
            size: 32_000_000_000,
            connection_bus: "usb".into(),
            removable: true,
            drive: Some("/org/freedesktop/UDisks2/drives/Stick".into()),
            partition_table: Some(format!("{OBJ}sdb")),
            ..Default::default()
        };
        Ok(vec![
            BlockDevice {
                label: "Games".into(),
                usage: "filesystem".into(),
                fs_type: "ntfs".into(),
                has_filesystem: true,
                partition_number: 1,
                ..stick("sdb1")
            },
            BlockDevice {
                label: "Vault".into(),
                usage: "crypto".into(),
                fs_type: "BitLocker".into(),
                encrypted: true,
                partition_number: 2,
                ..stick("sdb2")
            },
        ])
    }
    fn mount(&self, object: &str, options: &MountOptions) -> std::io::Result<PathBuf> {
        let call = format!("mount {} {}", short(object), options.options);
        self.calls.lock().unwrap().push(call.trim_end().to_string());
        Ok(self.mount_at.clone())
    }
    fn unmount(&self, object: &str) -> std::io::Result<()> {
        self.calls.lock().unwrap().push(format!("unmount {}", short(object)));
        Ok(())
    }
    fn lock(&self, object: &str) -> std::io::Result<()> {
        self.calls.lock().unwrap().push(format!("lock {}", short(object)));
        Ok(())
    }
    fn unlock(&self, object: &str, passphrase: &str) -> std::io::Result<String> {
        self.calls.lock().unwrap().push(format!("unlock {} {passphrase}", short(object)));
        if passphrase == "right" || passphrase == KEY {
            Ok(format!("{OBJ}dm0"))
        } else {
            Err(std::io::Error::other("Error unlocking /dev/sdb2: Failed to activate device: Operation not permitted"))
        }
    }
    fn eject(&self, drive: &str) -> std::io::Result<()> {
        self.calls.lock().unwrap().push(format!("eject {drive}"));
        Ok(())
    }
    fn power_off(&self, drive: &str) -> std::io::Result<()> {
        self.calls.lock().unwrap().push(format!("power off {drive}"));
        Ok(())
    }
}

fn config() -> Config {
    let mut c = common::test_config();
    c.layout.sidebar = true;
    c.sidebar.sections = vec![SidebarSection::Devices];
    // The sandbox turns device rows off; this test opts back in (with the fake backend).
    c.sidebar.unmounted_devices = UnmountedDevices::All;
    c
}

/// The sidebar line labelled `label` in the last frame.
fn line(a: &App, label: &str) -> Option<soyuz_tui::view::sidebar::SidebarLine> {
    a.view()?.sidebar.lines.iter().find(|l| l.label == label).cloned()
}

fn sidebar_cursor(a: &App) -> Option<String> {
    a.view()?.sidebar.lines.iter().find(|l| l.cursor).map(|l| l.label.clone())
}

fn menu_labels(a: &App) -> Vec<String> {
    let Some(m) = a.view().and_then(|v| v.menu.menu.as_ref()) else { return Vec::new() };
    m.items
        .iter()
        .map(|i| match i {
            MenuItem::Entry(e) => e.label.clone(),
            MenuItem::Separator => "-".into(),
        })
        .collect()
}

/// The text of the open dialog's passphrase field and its error.
fn field(a: &App) -> Option<(String, Option<String>)> {
    let d = a.view()?.dialog.as_ref()?;
    match &d.fields.first()?.kind {
        FieldVmKind::Text { text, .. } => Some((text.clone(), d.error.clone())),
        _ => None,
    }
}

fn called(calls: &Calls, what: &str) -> bool {
    calls.lock().unwrap().iter().any(|c| c == what)
}

#[test]
fn devices_list_unlock_and_mount() {
    let run = common::relaunch(module_path!(), "devices_list_unlock_and_mount", |data| {
        std::fs::create_dir_all(data.join("mnt")).unwrap();
    });
    if let Run::Child(data) = run {
        unlock_and_mount(&data);
    }
}

fn unlock_and_mount(data: &Path) {
    let calls: Calls = Arc::default();
    devices::set_desk_backend(Box::new(Fake { calls: calls.clone(), mount_at: data.join("mnt") }));
    let mut app = common::new_app(config(), data);
    let title = soyuz_appcore::tr!("dialogs-unlock-title", name = "Vault");
    let mut steps = vec![
        Step::new(
            "the device rows under the drives",
            |a| {
                line(a, "Games").is_some_and(|l| {
                    l.kind == LineKind::Device { state: CandidateState::Unmounted, busy: false } && l.right == "NTFS"
                }) && line(a, "Vault").is_some_and(|l| {
                    l.kind == LineKind::Device { state: CandidateState::Locked, busy: false } && l.right == "BitLocker"
                })
            },
            key(KeyCode::F(12)),
        ),
        Step::new("the sidebar focused", |a| a.key_context() == KeyContext::Sidebar, key(KeyCode::End)),
        Step::new(
            "the cursor on the locked device",
            |a| sidebar_cursor(a).as_deref() == Some("Vault"),
            key_mods(KeyCode::F(10), KeyModifiers::SHIFT),
        ),
        Step::new(
            "its menu: unlock, hide, copy the device path",
            |a| menu_labels(a) == ["Unlock…", "-", "Hide from sidebar", "Copy device path"],
            key(KeyCode::Esc),
        ),
        Step::new(
            "the menu closed, the cursor back on the locked device",
            |a| {
                menu_labels(a).is_empty()
                    && a.key_context() == KeyContext::Sidebar
                    && sidebar_cursor(a).as_deref() == Some("Vault")
            },
            key(KeyCode::Enter),
        ),
    ];
    let t = title.clone();
    steps.push(Step::new(
        "the unlock dialog, the field empty",
        move |a| {
            dialog_title(a).as_deref() == Some(t.as_str()) && field(a).is_some_and(|(f, e)| f.is_empty() && e.is_none())
        },
        key(KeyCode::Char('w')),
    ));
    common::type_text(&mut steps, "rong");
    steps.push(Step::new("bullets", |a| field(a).is_some_and(|(f, _)| f == "•••••"), alt('v')));
    steps.push(Step::new("revealed", |a| field(a).is_some_and(|(f, _)| f == "wrong"), alt('v')));
    steps.push(Step::new("hidden again", |a| field(a).is_some_and(|(f, _)| f == "•••••"), key(KeyCode::Enter)));
    let c = calls.clone();
    let t = title.clone();
    steps.push(Step::new(
        "a wrong passphrase: the error, the dialog open, the field wiped",
        move |a| {
            called(&c, "unlock sdb2 wrong")
                && dialog_title(a).as_deref() == Some(t.as_str())
                && field(a).is_some_and(|(f, e)| f.is_empty() && e.is_some_and(|e| !e.is_empty()))
        },
        key(KeyCode::Char('1')),
    ));
    // The recovery key without dashes: the core normalizes it before the backend sees it.
    let digits: String = KEY.chars().filter(|c| *c != '-').skip(1).collect();
    common::type_text(&mut steps, &digits);
    steps.push(Step::new(
        "48 digits typed",
        |a| field(a).is_some_and(|(f, _)| f.chars().count() == 48),
        key(KeyCode::Enter),
    ));
    let c = calls.clone();
    let mnt = data.join("mnt");
    steps.push(Step::new(
        // Linux pumps the mount: the pane opens the mounted folder (and takes the keyboard).
        "unlocked: the dialog closed, the volume mounted (and opened on Linux)",
        move |a| {
            called(&c, &format!("unlock sdb2 {KEY}"))
                && called(&c, "mount dm0")
                && dialog_title(a).is_none()
                && (!cfg!(target_os = "linux") || common::at(a, &mnt))
        },
        key(KeyCode::F(12)),
    ));
    steps.push(Step::new("the sidebar again", |a| a.key_context() == KeyContext::Sidebar, key(KeyCode::End)));
    steps.push(Step::new("on the last device", |a| sidebar_cursor(a).as_deref() == Some("Vault"), key(KeyCode::Up)));
    steps.push(Step::new(
        "the cursor on the stick",
        |a| sidebar_cursor(a).as_deref() == Some("Games"),
        key(KeyCode::Enter),
    ));
    let c = calls.clone();
    steps.push(Step::new("the stick mounted", move |_| called(&c, "mount sdb1"), ctrl('q')));
    common::run_script(&mut app, steps, 100, 30);
    // Never a real device call: everything the fake saw, in order.
    let seen = calls.lock().unwrap().clone();
    assert_eq!(seen[0], "unlock sdb2 wrong", "{seen:?}");
    common::ok();
}

#[test]
fn unlock_dialog_cancels_with_esc() {
    let run = common::relaunch(module_path!(), "unlock_dialog_cancels_with_esc", |_| {});
    if let Run::Child(data) = run {
        let calls: Calls = Arc::default();
        devices::set_desk_backend(Box::new(Fake { calls: calls.clone(), mount_at: data.join("mnt") }));
        let mut app = common::new_app(config(), &data);
        let mut steps = vec![
            Step::new("the locked device", |a| line(a, "Vault").is_some(), key(KeyCode::F(12))),
            Step::new("the sidebar focused", |a| a.key_context() == KeyContext::Sidebar, key(KeyCode::End)),
            Step::new("on the locked device", |a| sidebar_cursor(a).as_deref() == Some("Vault"), key(KeyCode::Enter)),
            Step::new("the dialog", |a| field(a).is_some(), key(KeyCode::Char('x'))),
        ];
        steps.push(Step::new("typed", |a| field(a).is_some_and(|(f, _)| f == "•"), key(KeyCode::Esc)));
        steps.push(Step::new("closed", |a| dialog_title(a).is_none(), ctrl('q')));
        common::run_script(&mut app, steps, 100, 30);
        assert!(calls.lock().unwrap().is_empty(), "nothing was unlocked");
        common::ok();
    }
}

/// The hibernated-NTFS offer is the core's `Confirm` (both frontends), confirmed here with
/// the terminal UI's confirm dialog: it mounts read-only (through the fake backend).
#[test]
fn read_only_offer_is_a_confirm() {
    let run = common::relaunch(module_path!(), "read_only_offer_is_a_confirm", |_| {});
    if let Run::Child(data) = run {
        let calls: Calls = Arc::default();
        devices::set_desk_backend(Box::new(Fake { calls: calls.clone(), mount_at: data.join("mnt") }));
        let mut app = common::new_app(config(), &data);
        let dev = BlockDevice { object: format!("{OBJ}sdb1"), fs_type: "ntfs".into(), ..Default::default() };
        app.core.frontend.push(devices::offer_read_only(Box::new(dev), "Games".into()));
        let title = soyuz_appcore::tr!("app-device-hibernated-title", name = "Games");
        let c = calls.clone();
        let steps = vec![
            Step::new("the offer", move |a| dialog_title(a).as_deref() == Some(title.as_str()), key(KeyCode::Enter)),
            Step::new(
                "mounted read-only",
                move |a| called(&c, "mount sdb1 ro") && dialog_title(a).is_none(),
                ctrl('q'),
            ),
        ];
        common::run_script(&mut app, steps, 100, 30);
        common::ok();
    }
}
