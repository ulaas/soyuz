#!/usr/bin/env python3
"""Automated accessibility audit for Linux: reads the AT-SPI tree that Soyuz exposes
through AccessKit (accesskit_unix) and checks names, roles, states and values.

This is the Linux counterpart of scripts/uia-audit.ps1 and reads exactly what Orca gets.
It is not a replacement for a human screen-reader run (see docs/ACCESSIBILITY.md).

1. Creates a small demo tree in a temporary directory (the same file names the Windows
   demo fixture uses for the audited states) and runs the release build in portable mode
   (an empty `portable` file next to the binary, settings in `portable-data/`, both removed
   afterwards).
2. Turns on the session's accessibility flags (org.a11y.Status IsEnabled and
   ScreenReaderEnabled; accesskit_unix only activates while ScreenReaderEnabled is true)
   and restores their previous values at the end.
3. Drives the app with SOYUZ_SCRIPT only (no input is sent to the desktop) through six
   states, holding each one with an `until:<file>` step while this script walks the tree:
     main        details view of Documents/Reports, one file selected, folder tree, tabs
     palette     the command palette with a query
     settings    the settings window (General)
     appearance  the settings window (Appearance: dropdowns, sliders)
     confirm     the "delete permanently" confirmation (cancelled with Esc afterwards)
     progress    a throttled copy job with the activity panel
4. Checks every state (G1-G9 below, plus per-state checks) and prints a Markdown table.

Exit code: the number of failed checks (0 = clean). Warnings do not fail the run.

Usage:
    cargo build --release -p soyuz-app
    python3 scripts/atspi-audit.py [--report out.md] [--only main,confirm] [--dump els.jsonl]

Needs python3-gobject with the Atspi 2.0 typelib (at-spi2-core); pyatspi is not used.
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, Gio, GLib  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
PHASES = ["main", "palette", "settings", "appearance", "confirm", "progress"]

# AT-SPI roles that must carry a name (the UIA audit's "interactive" control types).
INTERACTIVE = {
    "push button", "button", "toggle button", "check box", "radio button", "combo box", "entry",
    "password text", "text", "list item", "tree item", "menu item", "check menu item",
    "radio menu item", "page tab", "slider", "spin button", "progress bar", "link",
    "switch", "table cell",
}
TEXT_INPUTS = {"entry", "password text", "text", "spin button"}
TOGGLES = {"toggle button", "check box", "radio button", "switch", "check menu item", "radio menu item"}
# AccessKit maps an untoggled toggle button to the role alone (no CHECKABLE state, unlike
# check boxes), so only these must expose a checkable / checked state.
CHECKABLES = TOGGLES - {"toggle button"}
BUTTONS = {"push button", "button", "toggle button"}
SELECTABLE_ITEMS = {"list item", "tree item", "page tab"}
# Private Use Area (icon fonts) and the replacement character.
GLYPH_ONLY = re.compile(r"^[-\U000f0000-\U000ffffd�\s]+$")

ST = Atspi.StateType
STATE_NAMES = {
    "focusable": ST.FOCUSABLE, "focused": ST.FOCUSED, "selectable": ST.SELECTABLE,
    "selected": ST.SELECTED, "checkable": ST.CHECKABLE, "checked": ST.CHECKED,
    "pressed": ST.PRESSED, "expandable": ST.EXPANDABLE, "expanded": ST.EXPANDED,
    "enabled": ST.ENABLED, "sensitive": ST.SENSITIVE, "showing": ST.SHOWING,
    "visible": ST.VISIBLE, "modal": ST.MODAL, "editable": ST.EDITABLE,
}


# ---------------------------------------------------------------------------
# Session accessibility flags (org.a11y.Status on the session bus)
# ---------------------------------------------------------------------------

def a11y_status_proxy():
    return Gio.DBusProxy.new_for_bus_sync(
        Gio.BusType.SESSION, Gio.DBusProxyFlags.DO_NOT_LOAD_PROPERTIES, None,
        "org.a11y.Bus", "/org/a11y/bus", "org.freedesktop.DBus.Properties", None)


def get_status(prop):
    v = a11y_status_proxy().call_sync("Get", GLib.Variant("(ss)", ("org.a11y.Status", prop)),
                                       Gio.DBusCallFlags.NONE, 5000, None)
    return bool(v.unpack()[0])


def set_status(prop, value):
    a11y_status_proxy().call_sync(
        "Set", GLib.Variant("(ssv)", ("org.a11y.Status", prop, GLib.Variant("b", value))),
        Gio.DBusCallFlags.NONE, 5000, None)


# ---------------------------------------------------------------------------
# Demo tree (a small subset of scripts/demo-fixture.ps1, same names)
# ---------------------------------------------------------------------------

def make_demo(root: Path, big_mb: int):
    files = {
        "Documents/Reports/Q3 mission report.md": "# Q3 mission report\n\nAll systems nominal.\n",
        "Documents/Reports/Launch checklist.txt": "1. Fuel\n2. Oxygen\n3. Countdown\n",
        "Documents/Reports/Budget 2026.csv": "item,amount\nfuel,1200\nfood,300\n",
        "Documents/Reports/Crew roster.md": "- Commander\n- Engineer\n- Pilot\n",
        "Documents/Notes/Ideas.md": "Docking adapter v2\n",
        "Documents/Recipes/Borscht.md": "Beets, cabbage, dill.\n",
        "Downloads/Export.csv": "a,b\n1,2\n",
        "Archive/Reports/Q3 mission report.md": "# Q3 mission report (draft)\n",
        "Archive/Reports/Launch checklist.txt": "1. Fuel\n",
        "Music/.keep": "",
        "Photos/.keep": "",
        ".soyuz-demo": "Generated by scripts/atspi-audit.py\n",
    }
    for rel, text in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    if big_mb > 0:
        with open(root / "Downloads/Telemetry dump.dat", "wb") as f:
            chunk = bytes(range(256)) * 4096  # 1 MiB
            for _ in range(big_mb):
                f.write(chunk)


def toml_str(s):
    return "'" + s + "'"


# ---------------------------------------------------------------------------
# AT-SPI snapshot
# ---------------------------------------------------------------------------

def find_app(pid):
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is None:
            continue
        try:
            if app.get_process_id() == pid:
                return app
        except GLib.Error:
            continue
    return None


def safe(f, default=None):
    try:
        return f()
    except (GLib.Error, AttributeError, TypeError):
        return default


def snapshot(app, max_nodes=5000):
    """Walk the whole tree below the application; returns (frames, elements)."""
    safe(lambda: app.clear_cache())
    out = []
    frames = []
    stack = [(app, -1, 0)]
    while stack and len(out) < max_nodes:
        acc, parent, depth = stack.pop()
        role = safe(acc.get_role_name, "?") or "?"
        if depth == 1 and role == "frame":
            frames.append(safe(acc.get_name, "") or "")
        ss = safe(acc.get_state_set)
        states = {k for k, v in STATE_NAMES.items() if ss is not None and ss.contains(v)}
        ifaces = set(safe(acc.get_interfaces, []) or [])
        e = {
            "idx": len(out), "parent": parent, "depth": depth, "role": role,
            "name": safe(acc.get_name, "") or "", "desc": safe(acc.get_description, "") or "",
            "states": sorted(states), "ifaces": sorted(ifaces), "labelled_by": False,
        }
        if "Value" in ifaces:
            v = safe(acc.get_value_iface) or safe(acc.get_value)
            if v is not None:
                e["value"] = safe(v.get_current_value)
                e["min"] = safe(v.get_minimum_value)
                e["max"] = safe(v.get_maximum_value)
        if "Component" in ifaces and ("focusable" in states or role in INTERACTIVE):
            comp = safe(acc.get_component_iface) or acc
            r = safe(lambda: comp.get_extents(Atspi.CoordType.WINDOW))
            if r is not None:
                e["rect"] = [r.x, r.y, r.width, r.height]
        if role in TEXT_INPUTS:
            rels = safe(acc.get_relation_set, []) or []
            e["labelled_by"] = any(safe(r.get_relation_type) == Atspi.RelationType.LABELLED_BY for r in rels)
        out.append(e)
        n = safe(acc.get_child_count, 0) or 0
        kids = []
        for i in range(n):
            c = safe(lambda i=i: acc.get_child_at_index(i))
            if c is not None:
                kids.append((c, e["idx"], depth + 1))
        stack.extend(reversed(kids))
    return frames, out


# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

results = []


def check(phase, cid, what, ok, details=(), warn=False):
    status = "pass" if ok else ("warn" if warn else "fail")
    results.append({"phase": phase, "id": cid, "what": what, "status": status, "details": list(details)})
    mark = {"pass": "ok  ", "warn": "warn", "fail": "FAIL"}[status]
    print(f"  [{mark}] {cid:<4} {what}")
    for d in list(details)[:12]:
        print(f"         - {d}")


def describe(e):
    r = e.get("rect")
    pos = f" @{r[0]},{r[1]} {r[2]}x{r[3]}" if r else ""
    return f"{e['role']} '{e['name']}'{pos}"


def has(e, st):
    return st in e["states"]


def generic(phase, frames, els):
    named_frames = [f for f in frames if f.strip()]
    check(phase, "G1", f"the window has a name ({', '.join(named_frames) or 'none'})", bool(named_frames))
    unnamed_focus = [e for e in els if has(e, "focusable") and not e["name"].strip()]
    check(phase, "G2", f"no unnamed keyboard-focusable elements ({len(unnamed_focus)} found)", not unnamed_focus,
          map(describe, unnamed_focus))
    inter = [e for e in els if e["role"] in INTERACTIVE]
    unnamed = [e for e in inter if not e["name"].strip()]
    check(phase, "G3", f"interactive controls are named ({len(inter) - len(unnamed)}/{len(inter)})", not unnamed,
          map(describe, unnamed))
    nosel = [e for e in els if e["role"] in SELECTABLE_ITEMS and not has(e, "selectable")]
    check(phase, "G4", f"list / tree / tab items expose selection state ({len(nosel)} without)", not nosel,
          map(describe, nosel))
    glyph = [e for e in els if e["name"] and GLYPH_ONLY.match(e["name"])]
    check(phase, "G5", f"no names made only of icon glyphs ({len(glyph)} found)", not glyph,
          [describe(e) + f" U+{ord(e['name'].strip()[0]):04X}" for e in glyph])
    # G6: unnamed interactive siblings of the same role cannot be told apart by ear.
    groups = {}
    for e in inter:
        if not e["name"].strip():
            groups.setdefault((e["parent"], e["role"]), []).append(e)
    dup = [g for g in groups.values() if len(g) > 1]
    check(phase, "G6", f"no duplicate unlabeled siblings ({len(dup)} groups)", not dup,
          [f"{len(g)} x {g[0]['role']} under #{g[0]['parent']}" for g in dup])
    # G7: toggles expose a checked / pressed state, tabs a selected state, combo boxes expandable.
    bad = []
    for e in els:
        if e["role"] in CHECKABLES and not ({"checkable", "checked", "pressed"} & set(e["states"])):
            bad.append(describe(e) + " (no checkable/checked/pressed)")
    tabs = [e for e in els if e["role"] == "page tab"]
    if tabs and not any(has(e, "selected") for e in tabs):
        bad.append(f"{len(tabs)} tabs, none selected")
    check(phase, "G7", f"check boxes, radio buttons and tabs expose their state ({len(bad)} problems)", not bad, bad)
    # G7e: combo boxes / tree items expose expandable. accesskit_atspi_common 0.18 (egui 0.36)
    # does not map AccessKit's `expanded` to AT-SPI states yet, so this is a warning only.
    noexp = [e for e in els if e["role"] == "combo box" and not ({"expandable", "expanded"} & set(e["states"]))]
    check(phase, "G7e", f"combo boxes expose expandable / expanded ({len(noexp)} without)", not noexp,
          map(describe, noexp), warn=True)
    # G8: text inputs have a label (own name or a labelled-by relation).
    inputs = [e for e in els if e["role"] in TEXT_INPUTS]
    nolabel = [e for e in inputs if not e["name"].strip() and not e["labelled_by"]]
    check(phase, "G8", f"text inputs are labelled ({len(inputs) - len(nolabel)}/{len(inputs)})", not nolabel,
          map(describe, nolabel))
    # G9: focusable, enabled elements are reachable: on screen with a size and sensitive.
    focusable = [e for e in els if has(e, "focusable") and has(e, "enabled")]
    unreach = [e for e in focusable if not has(e, "sensitive") or ("rect" in e and (e["rect"][2] <= 0 or e["rect"][3] <= 0))]
    check(phase, "G9", f"focusable elements are reachable ({len(focusable) - len(unreach)}/{len(focusable)})",
          not unreach, map(describe, unreach))


def per_phase(phase, els):
    if phase == "main":
        items = [e for e in els if e["role"] == "list item"]
        check(phase, "M1", f"file list rows are list items with names ({len(items)})",
              len(items) >= 3 and all(e["name"] for e in items))
        sel = [e["name"] for e in items if has(e, "selected")]
        check(phase, "M2", f"the selected file is reported selected ({', '.join(sel)})",
              any(n.startswith("Q3 mission report") for n in sel))
        tree = [e for e in els if e["role"] == "tree item"]
        check(phase, "M3", f"folder tree rows are tree items ({len(tree)})", len(tree) >= 1)
        tabs = [e for e in els if e["role"] == "page tab"]
        check(phase, "M4", f"tabs are page tabs, one selected ({len(tabs)})",
              len(tabs) >= 2 and sum(has(e, "selected") for e in tabs) == 1)
        buttons = [e["name"] for e in els if e["role"] in BUTTONS and e["name"]]
        for want in ("Back", "Forward", "Up", "Settings"):
            check(phase, "M5", f"toolbar button '{want}' is present", any(b.startswith(want) for b in buttons))
    elif phase == "palette":
        edits = [e for e in els if e["role"] in TEXT_INPUTS and e["name"]]
        check(phase, "P1", f"the palette search field is a named entry ({len(edits)})", len(edits) >= 1)
        items = [e for e in els if e["role"] == "list item" and re.search("copy", e["name"], re.I)]
        check(phase, "P2", f"palette results are named list items ({len(items)} match 'copy')", len(items) >= 1)
    elif phase == "settings":
        wins = [e["name"] for e in els if e["role"] in ("dialog", "window", "frame", "internal frame") and e["name"] and e["depth"] > 1]
        check(phase, "S1", f"settings is a named window / dialog ({', '.join(wins)})", len(wins) >= 1)
        toggles = [e for e in els if e["role"] in TOGGLES]
        unnamed = [e for e in toggles if not e["name"]]
        check(phase, "S2", f"switches have names ({len(toggles) - len(unnamed)}/{len(toggles)})",
              len(toggles) >= 1 and not unnamed, map(describe, unnamed))
    elif phase == "appearance":
        named = [e for e in els if e["role"] in ("combo box", "slider", "spin button", "entry")]
        unnamed = [e for e in named if not e["name"]]
        check(phase, "A1", f"combo boxes, sliders and fields have names ({len(named) - len(unnamed)}/{len(named)})",
              len(named) >= 1 and not unnamed, map(describe, unnamed))
    elif phase == "confirm":
        dlg = [e for e in els if e["role"] in ("dialog", "alert") and e["name"]]
        check(phase, "C1", f"the confirmation is a named (alert) dialog ({'; '.join(e['role'] + ': ' + e['name'] for e in dlg)})",
              len(dlg) >= 1)
        btn = [e["name"] for e in els if e["role"] in BUTTONS and re.search("Delete|Cancel", e["name"])]
        check(phase, "C2", f"its buttons are named ({', '.join(btn)})", len(btn) >= 2)
    elif phase == "progress":
        bars = [e for e in els if e["role"] == "progress bar"]
        check(phase, "R1", f"a job progress bar is present ({len(bars)})", len(bars) >= 1)
        bad = [e for e in bars if not e["name"] or e.get("value") is None or not (0 <= e["value"] <= 100)]
        vals = "; ".join(f"{e['name']}={e.get('value')}" for e in bars)
        check(phase, "R2", f"progress bars have a name and a 0-100 value ({vals})", bars and not bad,
              map(describe, bad))


# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--exe", default=str(REPO / "target/release/soyuz"))
    ap.add_argument("--demo", help="demo tree location (default: a new temporary directory)")
    ap.add_argument("--report", help="write the findings as Markdown to this file")
    ap.add_argument("--only", default="", help="comma-separated states (default: all)")
    ap.add_argument("--timeout", type=int, default=180)
    ap.add_argument("--keep-demo", action="store_true")
    ap.add_argument("--dump", help="write every element of every state as JSON lines")
    args = ap.parse_args()

    exe = Path(args.exe).resolve()
    if not exe.is_file():
        sys.exit(f"no binary at {exe} (cargo build --release -p soyuz-app)")
    exe_dir = exe.parent
    marker, data = exe_dir / "portable", exe_dir / "portable-data"
    if marker.exists() or data.exists():
        sys.exit(f"{exe_dir} already has a portable marker or portable-data/; move them away first.")
    only = [p for p in args.only.split(",") if p]
    phases = [p for p in PHASES if not only or p in only]

    work = Path(tempfile.mkdtemp(prefix="soyuz-atspi-"))
    demo = Path(args.demo) if args.demo else work / "SoyuzDemo"
    if demo.exists():
        if not (demo / ".soyuz-demo").exists():
            sys.exit(f"{demo} exists and is not a Soyuz demo tree; refusing to replace it.")
        shutil.rmtree(demo)
    make_demo(demo, 64)
    D = lambda rel="": str(demo / rel) if rel else str(demo)  # noqa: E731

    def hold(name):
        return [f"mark:phase-{name}", f"until:{work / ('go-' + name)}"]

    steps = ["size:1400x900", "wait:2500", "select:Q3 mission report.md", "wait:800"]
    if "main" in phases:
        steps += hold("main")
    if "palette" in phases:
        steps += ["command_palette", "wait:300", "keys:copy", "wait:900"] + hold("palette") + \
                 ["keys:{Escape}", "wait:200", "keys:{Escape}", "wait:300"]
    if "settings" in phases:
        steps += ["settings:general", "wait:1200"] + hold("settings") + ["settings:close", "wait:400"]
    if "appearance" in phases:
        steps += ["settings:appearance", "wait:1200"] + hold("appearance") + ["settings:close", "wait:400"]
    if "confirm" in phases:
        steps += ["focus_list", "select:Launch checklist.txt", "delete_permanent", "wait:1000"] + hold("confirm") + \
                 ["keys:{Escape}", "wait:600", f"expect:exists={D('Documents/Reports/Launch checklist.txt')}"]
    if "progress" in phases:
        steps += ["select_tab:2", "wait:1200", "select:Telemetry dump.dat", "copy_to_other_pane", "wait:2500"] + \
                 hold("progress") + ["jobs_cancel_all", "wait:1500"]
    steps.append("quit")

    config_toml = f"""schema = 1

[general]
start_location = {toml_str(D('Documents/Reports'))}
language = "en-US"

[appearance]
theme = "dark"

[behaviour]
restore_session = true

[jobs]
auto_open_activity = true

[sidebar]
mode = "both"
sections = ["favorites", "tree", "trash"]
"""
    session_toml = f"""version = 1
active_tab = 0
welcome_seen = true

[[tabs]]
dual = false
focused = 0
split = 0.5
[[tabs.panes]]
location = {toml_str(D('Documents/Reports'))}

[[tabs]]
dual = true
focused = 0
split = 0.5
[[tabs.panes]]
location = {toml_str(D('Downloads'))}
[[tabs.panes]]
location = {toml_str(D('Archive'))}
"""

    saved = {}
    proc = None
    log_lines = []
    try:
        # Inside the try: the finally block removes the marker and portable-data/ even
        # when writing them fails half-way.
        data.mkdir()
        marker.touch()
        (data / "config.toml").write_text(config_toml, encoding="utf-8")
        (data / "session.toml").write_text(session_toml, encoding="utf-8")
        # at-spi2-core 2.62+ dropped ScreenReaderEnabled (only IsEnabled remains).
        for prop in ("IsEnabled", "ScreenReaderEnabled"):
            try:
                saved[prop] = get_status(prop)
            except GLib.Error:
                print(f"session a11y: org.a11y.Status has no {prop} property")
        print("session a11y: " + ", ".join(f"{k}={v}" for k, v in saved.items())
              + " (turned on for the audit, restored afterwards)")
        for prop, val in saved.items():
            if not val:
                set_status(prop, True)

        env = dict(os.environ)
        env["SOYUZ_SCRIPT"] = ";".join(steps)
        env["SOYUZ_THROTTLE_MBPS"] = "2"
        env["RUST_LOG"] = "info,wgpu_core=warn,wgpu_hal=warn,naga=warn"
        proc = subprocess.Popen([str(exe)], cwd=exe_dir, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, errors="replace")

        def reader():
            for line in proc.stdout:
                log_lines.append(line.rstrip("\n"))
        threading.Thread(target=reader, daemon=True).start()
        deadline = time.time() + args.timeout

        for ph in phases:
            print(f"== {ph}")
            want = f"SOYUZ_SCRIPT: mark phase-{ph}"
            while not any(want in ln for ln in log_lines):
                if proc.poll() is not None or time.time() > deadline:
                    raise RuntimeError(f"the app did not reach phase {ph}")
                time.sleep(0.2)
            time.sleep(0.6)
            # AccessKit activates when ScreenReaderEnabled is on; the full tree follows a frame later.
            frames, els = [], []
            for _ in range(40):
                app = find_app(proc.pid)
                if app is not None:
                    frames, els = snapshot(app)
                if len(els) >= 20:
                    break
                time.sleep(0.3)
            if not els:
                raise RuntimeError(
                    "no Soyuz application found in the AT-SPI registry (AccessKit did not activate; "
                    "accesskit_unix 0.21 waits for org.a11y.Status.ScreenReaderEnabled, which "
                    "at-spi2-core 2.62+ no longer has; see docs/ACCESSIBILITY_AUDIT.md)")
            counts = {}
            for e in els:
                counts[e["role"]] = counts.get(e["role"], 0) + 1
            print(f"   {len(els)} elements: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items(), key=lambda kv: -kv[1])))
            if args.dump:
                with open(args.dump, "a", encoding="utf-8") as f:
                    for e in els:
                        f.write(json.dumps(dict(e, phase=ph), ensure_ascii=False) + "\n")
            generic(ph, frames, els)
            per_phase(ph, els)
            (work / f"go-{ph}").touch()

        try:
            proc.wait(60)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        time.sleep(0.2)
        text = "\n".join(log_lines)
        for m in re.finditer(r"SOYUZ_SCRIPT: expect (ok|FAILED)[^\n]*", text):
            check("script", "X1", m.group(0)[14:], m.group(1) == "ok")
        if "panicked" in text:
            check("script", "X2", "the app did not panic", False)
    except (RuntimeError, GLib.Error) as err:
        check("script", "X0", str(err), False)
    finally:
        if proc is not None and proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(5)
            except subprocess.TimeoutExpired:
                proc.kill()
        for prop, val in saved.items():
            try:
                if get_status(prop) != val:
                    set_status(prop, val)
            except GLib.Error as err:
                print(f"could not restore {prop}={val}: {err}", file=sys.stderr)
        marker.unlink(missing_ok=True)
        shutil.rmtree(data, ignore_errors=True)
        if args.keep_demo and not args.demo:
            print(f"demo tree kept at {demo}")
        else:
            if not args.keep_demo:
                shutil.rmtree(demo, ignore_errors=True)
            shutil.rmtree(work, ignore_errors=True)

    failed = [r for r in results if r["status"] == "fail"]
    warned = [r for r in results if r["status"] == "warn"]
    print(f"\n{len(results)} checks, {len(failed)} failed, {len(warned)} warnings")
    md = [f"# AT-SPI audit ({time.strftime('%Y-%m-%d %H:%M')})", "",
          f"{len(results)} checks, {len(failed)} failed, {len(warned)} warnings.", "",
          "| State | Check | Result | What |", "|---|---|---|---|"]
    for r in results:
        res = "pass" if r["status"] == "pass" else f"**{r['status']}**"
        md.append(f"| {r['phase']} | {r['id']} | {res} | {r['what'].replace('|', '/')} |")
    for r in failed + warned:
        if r["details"]:
            md += ["", f"## {r['phase']} {r['id']}", ""] + [f"- `{d}`" for d in r["details"]]
    if args.report:
        Path(args.report).write_text("\n".join(md) + "\n", encoding="utf-8")
        print(f"report: {args.report}")
    sys.exit(min(len(failed), 255))


if __name__ == "__main__":
    main()
