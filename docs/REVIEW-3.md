# Review 3 (2026-11-05, branch phase-q)

A read-only review of the whole codebase by three reviewers: correctness and data
safety, performance, and user experience with typography. Each finding below is a
reviewer's claim until its fix is committed: the fix first confirms it against the code.

## Status

Fixed unless listed here. Findings below keep the reviewers' wording and line numbers from
before the fixes.

**Bugs: not fixed**
- 12 (a directory swapped for a junction during a Windows tree delete): needs a handle-relative
  tree delete. Checking first would only narrow the race.
- 15, the cross-process part: two instances changing different settings within the watcher's
  debounce can still lose one. Unknown keys inside `[[array]]` entries and inline tables are still
  rewritten whole.
- 17: a listing that stops partway warns with a notification; there is no banner with Retry.
- `kinds/sync.rs` overwrites are still `Irreversible`.

**Performance: not done**
- 12 (cell text cache): measured at about 38 µs per frame for 50 rows, too little to justify a
  staleness risk.
- 11: rows still make their own hash lookups for git status and tags, one per row.
- 15 (rubber band).
- 7: the OS accent and mode are still polled (3 s focused, 15 s unfocused), so a theme flip can take
  up to 15 s to show in an unfocused window.
- 6: a change touching only `refs/` waits for the 15 s fallback.

**UX: not done**
- 8: the welcome card does not dismiss itself after its tips were used.
- Ideas: conflict mnemonics and an "Identical" tag, omnibar ghost completion, type to filter in Open
  with, tips as a status-bar strip.

**Known limits**
- At `font_size` 17 the last Properties tab is cut off, and with a +8 offset all of them run past the
  window. The search, activity, tool, AI and preview surfaces got the font pass but were not checked by
  screenshot at the largest size.
- The terminal UI's notifications have no buttons: Ctrl+Z undoes the newest entry there, and hiding
  system folders still asks.

**Behaviour to review**
- A folder that has gone missing lands on its nearest existing ancestor with a notification. A path
  typed into the address bar or the palette that does not exist keeps its error page (owner decision).
- In a narrow window the filter field shrinks, then folds into a button until it is used; every other
  toolbar button stays, and below that the path field gives way (owner decision).
- `[git] refresh_secs` defaults to 15 instead of 5 (owner decision), now a fallback behind a watcher on
  the git directory.
- A remote transfer keeps a `.<name>.soyuzpartinfo` note beside a partial, and a source that changes
  during a transfer fails instead of committing.

**Verification**
- Windows: `cargo test --workspace`, workspace clippy, fmt and the banned-name grep pass.
- Linux: only `clippy -p soyuz-fs --lib` as a cross-check. The freedesktop trash tests added here, the
  git watcher and the theme thread have not run on Linux.
- Most new regression tests were not run against the unfixed code.
- Only a person can check: toast Undo and Esc, crumb Shift+click and Ctrl+click, the rename bubble's flip,
  Inter Tabular's crispness at 125 % and 150 %, and a real Start menu shortcut in Open with.

## Bugs and data safety

Read-only review of soyuz-jobs, soyuz-fs, soyuz-archive, soyuz-core config, soyuz-appcore, soyuz-vfs transfers, plus the two newest features (`models/file_browser.rs`, `models/requests/open_with.rs`). Items already in CLAUDE.md "Known gaps", REVIEW-1 and REVIEW-2 are not repeated. No files in the repo were changed; no cargo or git write commands were run. Paths are relative to `E:\source\private\soyuz-public\crates\`.

Severity order. Each finding: scenario → what goes wrong → fix; effort S/M/L; confidence.

---

### 1. Saving any setting replaces a config.toml that failed to parse with the defaults — HIGH, confirmed, S

`soyuz-core/src/config/preserve.rs:20` (`let Ok(mut doc) = existing.parse() else { return Ok(fresh) }`), called unconditionally from `soyuz-appcore/src/models/settings.rs:293-299`; the defaults fallback at load is `soyuz-appcore/src/core.rs:689-692`.

Scenario: the user hand-edits `config.toml` and leaves a syntax error. Startup: load fails, the app runs on `Config::default()` and shows the parse-error toast. The user then toggles any setting (or, since 66acd82, just picks an app in Open with… with "Always"/remember on — `open_with::remember` → `update_config` → `flush`). `flush` reads the broken text, `to_string_preserving` cannot parse it and silently returns the pretty-printed defaults plus that one change, and `write_atomic` overwrites the file. Keymap overrides, bookmarks, fonts, comments: all gone; no backup exists because `migrate_file` also bailed before writing one. Variant: the file parses as TOML but fails serde (`theme = "nope"`): comments survive but every customized key is rewritten to its default.

Fix: `to_string_preserving` returns `Err` when `existing` is non-empty and unparsable; `settings::flush` refuses to save while the active config was not loaded from the file (flag set in `load_config` / `config_watch::handle`) and toasts "fix config.toml first"; or at least copy the file to `config.toml.broken-<ts>` before writing.

### 2. `copy_symlink` deletes the destination before it can create the link, and ignores the overwrite condition — HIGH, confirmed, S

`soyuz-fs/src/copy.rs:374-394`; reached from `soyuz-jobs/src/kinds/copy.rs:343` (Overwrite → `copy_file(.., Overwrite::Always)` → `copy_symlink`; moves never follow links, copies don't by default).

Scenario: copy a symlink over an existing regular file, answer Overwrite (or "overwrite all" is remembered). `delete_file(dst)` runs first (permanent, not trash), then `links::symlink`. On Windows without Developer Mode / symlink privilege `CreateSymbolicLinkW` fails (error 1314); on unix it fails on FAT/exFAT mounts. Result: the user's file is gone and nothing replaces it; the record is `Irreversible`. Also every policy other than `Never` (`IfNewer` / `IfLarger` / `IfDifferentSize`) replaces unconditionally for links because the comparison at `copy.rs:180-186` is bypassed.

Fix: create the link under a `temp_sibling` name and `rename(tmp, dst, Replace)`; remove the temp on failure; apply the `Overwrite` comparison to links too.

### 3. Extracting a symlink or hard link over an existing file deletes the file first, then may fail to create the link — HIGH, confirmed, S

`soyuz-archive/src/extract.rs:653-658` (`remove_existing` then `make_symlink`), `:698-703` (hard link; the `fs::copy` fallback can fail as well).

Scenario: Windows without symlink privilege. Archive has symlink entry `notes.txt -> ../x`; `dest/notes.txt` exists; the jobs sink shows a plain file conflict, the user picks Overwrite: `remove_existing` deletes `notes.txt`, `symlink_file` fails with 1314, the entry is reported failed, the original is gone (not trashed, not undoable). Regular files are protected by temp + rename; links are not.

Fix: create the link under the `.soyuz-extract-<pid>-<n>.part` temp name and `rename_into_place` it (remove an existing empty dir only after the link exists), or probe symlink capability once per extraction and fail the entry before touching the destination.

### 4. Resumable remote transfers resume a stale `.soyuzpart` and never verify the result — HIGH, confirmed, M

`soyuz-vfs/src/transfer.rs:81-90` (resume decision: only `offset > fc.size || !range_read` restarts), `:94-117` (pump; no `done == fc.size` check before commit/rename).

Scenario A: an upload fails at 4 MB of 10 MB (partial kept by design). The user edits the source locally and retries: the first 4 MB are old content, the rest new, renamed into place and reported as success — silent corruption. Scenario B: a `.name.soyuzpart` left by an earlier failed copy of a *different* file with the same name is resumed on top of; with `offset == fc.size` the source is not even read. Scenario C: the source shrank between listing and copy: `done < fc.size`, the short file is still committed. Nothing records which source (size, mtime) a partial belongs to.

Fix: record the source size + mtime beside the part (sidecar or name-encoded); resume only on a match, else `abort(false)` and restart; after the pump require `done == fc.size` (and the committed size when the destination can stat) before the rename.

### 5. Delete / Rename / Enter act on an item the live filter has hidden — MEDIUM-HIGH, confirmed, S

`soyuz-appcore/src/state/pane.rs:495-508` (`selected_entries` falls back to `self.cursor` resolved with `self.entry(c)`, which searches all `entries`, not `visible`), used by `ops/fileops.rs:324-349` (Delete), `:375-390` (`start_rename`), `ops.rs:346-366` (`open_selection`).

Scenario: click `a.txt` (selected + cursor). Type-to-filter "b": only b-files show; `set_filter` leaves `cursor` alone. Press Delete: no visible selected entry → cursor fallback → `a.txt` is trashed while not on screen; with `confirm_delete = false` (default since fcfb34a) there is no prompt. F2 and Enter take the same path. The test `filter_and_cursor` asserts `cursor_pos() == None` but never checks `selected_entries()`.

Fix: fall back to the cursor only when `cursor_pos().is_some()`; also clear or re-home the cursor in `set_filter` when it drops out of `visible`.

### 6. `trash_unsupported = permanent` plus the new no-confirm Delete deletes permanently without any prompt — MEDIUM, confirmed, S

`soyuz-appcore/src/ops/fileops.rs:340-349` (Trash mode consults only `confirm_delete`, default `false` since migration v4) and `:724-737` (`TrashUnsupportedPolicy::Permanent` auto-answers the prompt with `permanent: true, apply_to_all: true`); `soyuz-jobs/src/kinds/trash.rs:57-65` then runs `remove_path_quiet` and records `Irreversible`.

Scenario: a user set `[jobs] trash_unsupported = "permanent"` back when Delete still asked. Migration v4 flipped `confirm_delete` to false for everyone. Now a single Delete key on a FAT32 stick, an SMB share or any volume without a bin deletes the selection permanently with zero confirmation, bypassing `confirm_permanent_delete = true`. Same for the `remove_source` step of a move off such a volume (`kinds/remote.rs:478`).

Fix: when the policy is `Permanent`, still honour `confirm_permanent_delete` (ask once per job: the `TrashUnsupported` prompt, or the delete-confirm dialog up front when any selected path's volume reports `Unsupported`); or make the v4 migration leave `confirm_delete = true` when `trash_unsupported = permanent`.

### 7. Cancelled or aborted jobs leave no history: partial moves cannot be undone, and a remote replace trashes the local file before the copy — MEDIUM, confirmed, M

`soyuz-jobs/src/engine.rs:521-554` (`finish_job`: a history entry is created only for `Done`; `Cancelled` / `Failed` drop the records), `:568-570` (the crash journal entry is removed on every finish). `kinds/remote.rs:371-378` (`replace` → `trash_local(dst)` *before* `copy_file`).

Scenario A: cross-device Move of three folders; the user cancels during the third. Folders 1 and 2 are on the destination, their sources deleted (`kinds/mv.rs:150-182`), and nothing anywhere records it: Ctrl+Z says "nothing to undo", the activity history has no entry, Retry is unavailable. Same when the user picks Abort on an error prompt after 500 of 1000 files. Scenario B: paste a file from an SFTP server over a local file, answer Overwrite: the local original goes to the trash first, then the download fails or is cancelled (`discard_partial`): the destination is now missing, the `Trashed` record is dropped with the job, and the user has to find the file in the trash view by hand.

Fix: create a history entry for `Cancelled` / `Failed` jobs too when they produced records (label "partial", undoable), or keep the journal entry as a recovered job instead of `end`ing it. For the remote replace, trash the old file only after `copy_file` succeeded (the part + rename already makes the replace atomic).

### 8. The completion toast's "Undo" undoes whatever finished last, not the operation the toast names — MEDIUM, confirmed, M

`soyuz-appcore/src/ops/fileops.rs:841-843` (`.action(tr!("common-undo"), Action::Undo)`), `:564-580` (`undo` picks the newest `this_session && !undone && undoable` entry); toast actions carry a bare `Action` (`models/toasts.rs:20`).

Scenario: copy 3 files (toast with Undo for 7 s); within that window F2-rename a file (quiet job, still an undoable history entry); click Undo on the copy toast → the rename is reverted, the copies stay.

Fix: carry the `HistoryId` in the toast (`Action::UndoEntry(id)` or a `Cmd` payload) resolved in `on_finished`, dispatched through `history_action(app, h, Undo)`.

### 9. Windows `trash::list()` scans drive letters only; items trashed on folder-mounted volumes disappear from Soyuz — MEDIUM, confirmed, S

`soyuz-fs/src/trash/windows.rs:278-297` (`trash()` at `:187` uses the real volume root; `volumes.rs:546` already lists `volume_mount_paths()`).

Scenario: a volume mounted at `C:\mnt\data` without a letter. `trash(C:\mnt\data\x)` moves it to `C:\mnt\data\$Recycle.Bin\<SID>\$R…`, but `list()` / `empty()` iterate `A:`–`Z:` only, so the Trash view never shows it and Empty Trash never purges it; the user believes it was deleted for good (undo via the history entry still works).

Fix: also iterate `volume_mount_paths()` in `list()` (dedup, same drive-type filter).

### 10. Spanned zip directory rebuild: unchecked u64 arithmetic and an allocation sized by the end record — MEDIUM, confirmed, S

`soyuz-archive/src/reader/spanned.rs:167` (`disks.start(rec_disk) + rec_off`), `:186` (`dir.offset += …`), `:193-196` (`dir.offset + dir.size > total`, then `vec![0u8; dir.size]` in `read_at`), `:291` (`start + offset`).

Scenario: a crafted `x.z01` + `x.zip` pair whose zip64 EOCD says offset 0x200, size 0xFFFF_FFFF_FFFF_FF00. Release builds have no overflow checks; the sum wraps to 0x100 and passes the bounds check; `vec![0u8; ~2^64]` aborts the process (allocation failure, not a catchable panic). Reachable by previewing or opening the file. Even without the wrap, `dir.size` up to the whole set size (GBs) is allocated at once.

Fix: `checked_add` wherever file-supplied offsets are combined; cap `dir.size` (e.g. ≤ 256 MiB and ≤ `total - offset`) before `read_at`; same for `rec_off` and entry offsets.

### 11. `safe_dirs` is not invalidated when a symlink entry replaces a directory: later entries write through the link — MEDIUM, confirmed, S

`soyuz-archive/src/extract.rs:435` (dir added to `safe_dirs`), `:653-658` (symlink replaces the dir; `safe_dirs` untouched), `:377-379` (`ensure_dir` trusts `safe_dirs` and skips the symlink check), `:298-309` / `:475-485` (`finish` / `set_mode` / `set_file_times` follow the link).

Scenario: entries `a/`, `a` (symlink → `b`), `a/evil.txt`, extracted with Overwrite::Always or a remembered "overwrite all". `a/` creates `dest/a`; the symlink entry wins the conflict, the empty dir is removed and `dest/a -> b` is created (the lexical check allows ending at `b`); `a/evil.txt` calls `ensure_dir(dest/a)`, which returns early, and `File::create(dest/a/.soyuz-extract-…)` follows the link. If `dest/b` is a pre-existing user symlink to `/home/user`, the file lands outside the destination; inside dest it still breaks the documented "never writes through a symlink" guarantee.

Fix: whenever `remove_existing` removes a directory, drop it and its descendants from `safe_dirs` and `dir_remap`; in `ensure_dir` re-check `symlink_metadata` for the last component when `link_rels` contains that rel path.

### 12. Windows `remove_tree` descends by path: a directory swapped for a junction mid-delete gets its target emptied — MEDIUM, plausible, M

`soyuz-fs/src/delete.rs:224-266` → `list.rs:290` (`read_dir(&path)`) → `sys/windows/ntdir.rs:131` (opened with `FILE_FLAG_BACKUP_SEMANTICS` only, no `FILE_FLAG_OPEN_REPARSE_POINT`). `delete.rs:6-7` promises links are never followed; the unix side uses `O_NOFOLLOW` dirfds, Windows has no equivalent here.

Scenario: between the parent listing (which saw `sub` as a Dir) and `read_dir(tree\sub)`, another process replaces `sub` with a junction to `C:\Users\me\Documents`; the children are deleted by path, then `RemoveDirectoryW` removes the junction. Needs a race or a hostile process, hence plausible.

Fix: open each directory with `FILE_FLAG_OPEN_REPARSE_POINT` and check `FileAttributeTagInfo` has no reparse tag before enumerating, or enumerate/delete via handles relative to the parent.

### 13. Lane probing runs on the scheduler thread: one job aimed at a dead network path stalls admission of every job — MEDIUM, plausible, S

`soyuz-jobs/src/engine.rs:586-618` (`compute_lanes` → `device_for` → `probe.device_of` for every `PerDevice` job before `admit`), `:878-901` (`scheduler_main` runs them in sequence; sampling, `reap_threads` and the journal tick wait too). `is_remote_root` only filters `scheme://` URIs, not UNC paths or mapped drives.

Scenario: paste into `\\nas\share` while the NAS is asleep. `GetVolumePathNameW` / `GetVolumeInformationW` on the dead UNC path takes 30-60 s; during that time a local Copy or Delete submitted afterwards stays "Probing", no speed samples are taken and the crash journal is not written. The UI thread is not blocked, but the engine looks frozen.

Fix: compute lanes per job on the worker (or a probe pool) with a timeout, admitting with `Lane::Global` on timeout; or give `device_for` a cached answer per volume root and a deadline.

### 14. "Copy to… / Move to…" with a relative destination moves the files under the process cwd — MEDIUM, confirmed path / plausible impact, S

`soyuz-appcore/src/models/dialogs.rs:666-673` (`PathBuf::from(expand_env(text))`, no absolute/exists check), `ops/fileops.rs:446-495` (`valid_target` checks containment only); `soyuz-jobs/src/planner.rs:80` (`nearest_existing` resolves against cwd) and `kinds/copy.rs:55-66` (`ensure_dir` creates the missing folder).

Scenario: the user types `Backup` meaning a sibling folder → `<cwd>\Backup` is created (launch dir, exe dir or System32) and the files are moved there; the toast says "Moved 12 items to Backup". Undoable, but the files are effectively lost until the user finds them.

Fix: refuse relative destinations with an inline error in `TransferModel::confirm`, or resolve them against the sources' parent folder.

### 15. Config writes are whole-file last-writer-wins and drop every key the running build does not know — MEDIUM, confirmed (timing part plausible), M

`soyuz-core/src/config/preserve.rs:31-34` (`stale` = every key missing from the serialized config is removed); `soyuz-appcore/src/models/settings.rs:293` (merges the whole in-memory config into whatever is on disk) and `services/config_watch.rs:118-122, 170-171` (`apply_config` replaces `app.config` wholesale).

Scenario A: a newer Soyuz wrote `[media] foo = …`; the user opens the same config dir with an older build (portable copy, downgrade) and changes one setting: all newer keys and their comments disappear. Mistyped keys are silently removed instead of left for the user to fix. Scenario B: two instances (GUI + TUI over SSH) change different settings within the ~120 ms watcher debounce: the second save writes the first setting back to its old value. Scenario C: a setting toggled in the UI, an external edit arrives before `SAVE_DELAY` elapses → `pump` replaces `app.config`, the toggle silently reverts, and `flush` writes the external content back.

Fix: keep unknown keys (only remove known-schema map entries the user deleted, e.g. `keys.bindings`, `bookmarks`); three-way merge in `flush` (apply only keys changed since the last loaded config onto the freshly read file) and apply the same delta on top of an externally reloaded config.

### 16. File arguments ending in a backslash are quoted as `"C:\"`, which `CommandLineToArgvW` reads as an escaped quote — LOW-MEDIUM, confirmed, S

`soyuz-fs/src/launch/windows.rs:266, 292, 698` and `launch/mod.rs:556` (`/select,"C:\"`). `windows_quote_arg` (`mod.rs:457`) handles this but is not used on these paths.

Scenario: Open with… on a volume root (`plain()` yields `D:\`) with a program registered as `"app.exe" "%1"` → argv[1] becomes `D:"` merged with the following arguments; the program opens the wrong thing or nothing. Related: `.bat`/`.cmd` launch via `cmd.exe /c ""path""` (`windows.rs:609-617`) lets cmd expand `%VAR%` inside the file name (`50%OFF%.bat`).

Fix: double trailing backslashes before the closing quote in `expand_args` / `open_with` (reuse `windows_quote_arg`); spawn `.bat` with `Command::new(&p)` and no shell wrapper.

### 17. Listing errors are hidden, blank the pane, or remove a live entry — LOW-MEDIUM, code confirmed / impact plausible, M

`soyuz-appcore/src/state/pane.rs:309-316` (a `read_dir` error after some entries → `Ready` with only `log::warn`), `:284-296` (the same error on a soft relist clears `entries` and shows Failed), `services/lister.rs:233` (`Err(_) => removed.push(name)` on a re-stat; the Windows stat only falls back to `FindFirstFile` for sharing-violation / access-denied).

Scenario: an SMB share stalls mid-enumeration → half a folder is shown as complete. A transient error during an automatic post-job relist blanks a pane that was fine, losing selection and cursor. A watcher event on `\\nas\share\big.iso` whose stat fails with `ERROR_SEM_TIMEOUT` / `EIO` makes the existing file vanish from the pane until the next relist.

Fix: a `Partial(FsError)` status with a banner and Retry; on a failed soft relist keep the old entries and toast a warning; treat only `NotFound` as a removal in the patcher.

### 18. Finishing a cut-paste wipes a clipboard set copied meanwhile — LOW, confirmed, S

`soyuz-appcore/src/ops/fileops.rs:821-826` (`AfterJob::ClearClipboard` → unconditional `clipboard.clear()` + `clear_cut()`), set up at `:432-437`.

Scenario: Cut A, paste into a slow destination; while the Move runs, Copy B; the move finishes → the internal clipboard is cleared while the OS clipboard still holds B: Soyuz says "nothing to paste", Explorer still pastes B.

Fix: stamp the clipboard with a generation stored in `AfterJob::ClearClipboard { gen }` and clear only when unchanged.

### 19. Small fs correctness items — LOW, confirmed, S each

- `soyuz-fs/src/copy.rs:246-259, 264-278`: `temp_fits` counts chars, not UTF-16 units. A new file named 120 emoji + `.txt` (≈250 units, 130 chars) is staged with the full temp name → 284 units → `create_new` fails `ERROR_FILENAME_EXCED_RANGE`, although the in-place / hashed fallbacks exist for this case. Fix: `encode_wide().count()` on Windows.
- `soyuz-fs/src/trash/freedesktop.rs:101-122`: a 250-byte name (legal on ext4) gives a 260-byte `info/<name>.trashinfo` → `ENAMETOOLONG`, trash fails. Fix: truncate the stem (char boundary) for the id; only `Path=` must hold the real name.
- `soyuz-fs/src/trash/freedesktop.rs:60-98`: no "already in the bin" guard (Windows has `path_overlaps` at `windows.rs:196`); trashing `~/.local/share/Trash/files/x` again orphans `info/x.trashinfo`. Fix: reject paths under `bin.root`.
- `soyuz-fs/src/trash/windows.rs:187, 239-242`: `GetVolumePathNameW` does not resolve junctions, so `C:\link\file` (junction → `D:`) picks `C:\$Recycle.Bin`; `MoveFileExW` fails with error 17 shown raw ("os error 17"). Fix: canonicalize before `volume_root`; map error 17.

### 20. Undo and Open-with bookkeeping edge cases — LOW, confirmed, S

- `soyuz-jobs/src/undo.rs:109-134`: `Moved` / `Renamed` records carry no identity stamp (only `Copied` / `Extracted` / `Created` do). Move `report.docx` from Desktop to Archive, later another app saves a new `report.docx` into Archive, Ctrl+Z moves the *new* file to Desktop. Nothing is deleted, but undo touches a file the job never produced. Fix: stamp `to` for moves/renames like the others and refuse on mismatch.
- `soyuz-core/src/config/schema.rs:713-742` + `soyuz-appcore/src/models/requests/open_with.rs:301-318`: `app_for` returns the first rule that matches, `set_default(ext)` only removes `ext` from rules whose normalized key is exactly `ext`. With a user rule `extensions = ["tar.gz"]` → 7-Zip, Open with… on `x.tar.gz`, pick VLC with "Always use this app for .gz files": a new `gz` rule is appended, but `app_for("x.tar.gz")` still returns the `tar.gz` rule, and the dialog keeps preselecting 7-Zip with "Always" ticked. The user's choice is silently ignored. Fix: in `set_default`, also drop rules that `matches()` the same name family (or make `app_for` prefer the most specific/newest rule); show the shadowing rule in the dialog.
- `soyuz-appcore/src/services/lister.rs:263-266` + `ops.rs:626-630`: sync-navigation probes carry no generation; a late `Probed { exists }` from a slow mirror drive navigates the other pane after the user already moved it elsewhere, pushing history. Fix: add `generation` to `Probe` / `Probed` as `Resolve` has.

---

### Also noted (not ranked, low)

- `soyuz-fs/src/watch.rs:573, 585`: `dispatch` still stats under the registry lock for ancestor/own-dir Remove events (REVIEW-2 #19 moved only the liveness stats). A dead UNC share stalls notify's thread and `Watcher::watch` / `unwatch` on the UI thread. Fix: collect ids under the lock, stat outside.
- `soyuz-vfs/src/transfer.rs:79-82` + `s3.rs:442-447`: direct (S3) writes ignore `replace`; the `!fc.replace` check at `:119-128` only runs for the temp-file path, so a key that appears between `settle()`'s stat and the PUT is overwritten and recorded as "created" (undo would then delete it). Fix: `stat_opt` right before `create_write`, or `If-None-Match: *`.
- `soyuz-appcore/src/services/watcher.rs:81-88` + `ops.rs:697`: a folder already gone at navigation time (session restore on an unplugged drive) never gets a `Lost` event, so no ancestor fallback or toast; plugging the drive back relists nothing until F5.
- `soyuz-appcore/src/services/instance.rs:317, 341`: `read_line` on the instance pipe is unbounded. Fix: `.take(64 * 1024)`.
- `soyuz-jobs/src/kinds/sync.rs:51`: sync overwrites go through `transfer_tree(.., Overwrite)` and are recorded `Irreversible`: the replaced side is lost permanently (no trash), unlike every other overwrite path that at least prompts. Design choice today; worth trashing the replaced file first like `kinds/remote.rs` does.

### Checked and found sound (not reported)

Jobs: lock ordering and no I/O or sends under `state`; prompt channel draining and cancel-while-prompting; pause/resume/cancel races around admission (a job cancelled between `admit` and `spawn_worker` finishes `Cancelled` cleanly); `history_busy` double-undo guard; partial-undo record dropping (`without`); stamp checks on `Copied` / `Extracted` / `Created`; cross-device move deletes exactly `tree.copied`; `move_prim` copy-back refuses to delete the source when anything was skipped; rename dependency / park / unpark logic; `remove_created` never deletes a non-empty folder permanently without the prompt; `edit_archive` trash-then-rename with restore on failure; `TrashUnsupported` default `Ask`.

fs: temp-file-and-rename overwrite with READONLY restore, `COPY_FILE_FAIL_IF_EXISTS` / `create_new`, block-clone and sparse paths, `is_case_variant`, unix `remove_tree` `O_NOFOLLOW` dirfds, `$I` v1/v2 bounds, `.lnk` and reparse-buffer parsers, `exec_tokens` / field-code quoting on Linux, `expand_args` URL safety, FILETIME saturation, lossless non-UTF-16 wide paths.

Archive / config / vfs: zip-slip / absolute / drive-letter / `..` handling, symlink chain rejection, declared and actual bomb limits, CAB / ISO / UDF / RPM / deb header bounds (`get()`-guarded, 64 MiB directory cap), 7z empty-entry counting, `KeyChord` parsing, `#[serde(default)]` on every config struct, `write_atomic` fsync, migrations (backup never overwritten, idempotent), Quick share path resolution (token, canonical prefix, GET/HEAD only), remote listing name sanitization (`check_child_name`), URI password handling.

appcore: name-keyed selection across sort / patch / relist; `NavHistory` and tab-index clamping; job meta / affected-path routing across windows; `store_file` corrupt-file protection; `split.rs` never overwrites; `dupes.rs` keeps one copy per group and verifies existence; `updates.rs` verifies SHA-256 at download and install; `rename/mod.rs` duplicate / case-only / cycle detection; `file_browser.rs` listing thread and stale-receiver handling; `open_with.rs` `choose` validation (absolute + is_file), `adopt` dedup, `listed_apps` / `preselect` logic (other than item 20).

## Performance

### Findings (ranked by user-visible impact)

1. **Natural sort allocates two Strings per comparison for names with digits**
   `crates/soyuz-core/src/naming.rs:80-91` (`take_digits` returns `String`), called from `natural_cmp_inner` at `:54-55`; also `crates/soyuz-core/src/sort.rs:124-131` (`ext_key`/`kind_key` allocate per comparison for the Kind/Extension columns).
   Today: sorting a 100k-photo folder (`IMG_0001.jpg`...) is ~1.7M comparisons x 2 heap allocations, on the UI thread (`Pane::resort` `pane.rs:371`, soft-relist sort `pane.rs:292`, per-chunk sort `listing.rs:15`). Column-header clicks and every relist stall for hundreds of ms.
   Fix: compare digit runs as `&str` slices via byte offsets (no `String`); compare extensions with a case-folding iterator instead of `to_lowercase()`. Effort S. Risk low (the sort bench and tests pin behaviour). Confirmed.

2. **Streaming listings redo O(n) work every frame while a big folder loads**
   `crates/soyuz-appcore/src/state/pane.rs:387-394` (`flush_incoming`) -> `listing.rs:27-45` (full merge allocates a new `Vec<EntryInfo>` and moves all n x 216 B whenever the chunk does not sort after the tail, i.e. any sort other than name-asc on NTFS) -> `after_change` -> `NameIndex::rebuild` `pane.rs:570-584` (rehash all names) -> `recompute_visible`.
   Today: 100k entries streamed over ~1 s at 60 fps = 60 x O(100k) on the UI thread; the frame budget is blown while the list fills.
   Fix: merge/rebuild at most every ~100 ms (or when >= N entries are pending) and rebuild the index once on `done`; or keep entries in arrival order and sort a `Vec<u32>` index so merges never move 216 B records. Effort M. Risk med. Confirmed (magnitude plausible).

3. **A plain click allocates the whole visible key list**
   `crates/soyuz-app/src/ui/views/flow.rs:360-361`: `let order = pane.visible_keys(); pane.selection.click(&order, ...)`. `Selection::click` (`state/selection.rs:125-134`) uses `order` only for Shift ranges. Same pattern for Shift+arrow at `pane.rs:532` (noted in the handover).
   Today: every left click in a 100k folder allocates 100k `Arc<str>` (~5-10 ms of click latency).
   Fix: build `order` lazily only when `mods.range` (pass a closure or `Option`). Effort S. Risk low. Confirmed.

4. **"Calculate folder sizes" does a linear scan of `entries` per size update and resorts per frame**
   `crates/soyuz-appcore/src/services/search.rs:535` (`pane.entries.iter_mut().find(|e| &e.path == path)`), `:541` (`pane.resort()` when sorted by Size), `:543` (`pane.dirty = true` every pump).
   Today: up to 256 `Partial` events per frame x 100k path compares, plus a full resort and a full `compute_visible` each frame for the duration of the calculation in a large folder -> multi-second stutter.
   Fix: look entries up through `pane.index_of(file_name)` (O(1)); resort only on `Done` and throttle to ~4 Hz; mark dirty only when a visible row changed. Effort S. Risk low. Confirmed.

5. **Grouped views rebuild a translated label String for every entry on every recompute**
   `crates/soyuz-appcore/src/state/listing.rs:101-103` (`group_key` per entry -> `(u32, String)` + `tr!` RwLock lookup; `kind_label` `:240-241` also lowercases the extension), then sorts by String compare.
   Today: in a grouped 100k folder every type-to-filter keystroke, watcher patch, folder-size tick and streamed chunk costs ~300k allocations + a string sort (tens of ms).
   Fix: compute a small integer group id per entry once when `entries` change (cache a `Vec<u16>` next to them), intern labels (there are ~60), sort `vis` by `(order, id)`. Effort M. Risk low. Confirmed.

6. **`git status` is re-run every 5 s for every visible repository while idle**
   `crates/soyuz-appcore/src/services/git.rs:619-656`: `expired` after `refresh_secs` (default 5, `soyuz-core/src/config/defaults.rs:483`) while `touched` is within 3 s (`:29`), and `touched` is bumped by every `status_of` call, i.e. every painted frame. Spawns a thread + `git status --porcelain=v2` (`:369`) + `read_branch`, then rebuilds the `StatusIndex`. `models/tools.rs:498-499` adds a 500 ms repaint while a stale repo waits.
   Today: idle in a monorepo = a git subprocess every 5 s (CPU, disk, antivirus) with nothing changed.
   Fix: refresh on events only (the watcher already calls `git::invalidate`, `ops.rs:679`); keep the TTL as a slow fallback (30-60 s) or watch `.git/index`/`HEAD`; pause when unfocused. Effort S. Risk low-med. Confirmed.

7. **Theme watcher thread wakes every 1.5 s forever to read the registry and `read_dir` the themes folder**
   `crates/soyuz-app/src/theme/library.rs:134-145` loop (`poll_os` + `scan`), `scan` at `:166-178` stats every file.
   Today: permanent idle wakeups + disk I/O even when minimized.
   Fix: watch the themes dir with `soyuz_fs::Watcher`; for the accent/app-mode use the OS change notification (or back off to >= 10 s when unfocused). Effort S-M. Risk low. Confirmed.

8. **`config.toml` is read, merged and written synchronously on the UI thread**
   `crates/soyuz-appcore/src/models/settings.rs:287-313` (`flush`: `read_to_string`, `to_string_preserving` via toml_edit, `write_atomic`), run from `pump` in `pump_services` (`core.rs:473`) 700 ms after any `update_config` (Ctrl +/-, player volume keys, tag rename, any settings toggle).
   Today: a one-off hitch (parse + serialize + fsync) in the middle of a frame; session/views already use `store_file::save_async`.
   Fix: hand the merge + write to the store worker with a config snapshot; report `flashed` back via the mailbox. Effort S. Risk low. Confirmed.

9. **Status bar recomputes selection stats every frame**
   `crates/soyuz-app/src/ui/shell.rs:148` -> `pane.selection_stats()` (`state/pane.rs:511-523`): iterates the `BTreeSet` and does a hash lookup per selected key.
   Today: after Ctrl+A in a 100k folder every frame (hover animations, spinners) spends ~100k lookups.
   Fix: cache `(count, bytes)` in `Pane` behind a selection generation + `vis_version`, or maintain the totals incrementally in `Selection`. Effort S. Risk low. Confirmed.

10. **Every frame with an active job re-snapshots all jobs under the engine lock**
    `crates/soyuz-appcore/src/services/jobs.rs:372-378` (`refresh_snapshots` whenever any job is active, not just at the 100 ms repaint cadence) -> `soyuz-jobs/src/engine.rs:1114-1119` (BTreeMap of positions + a `JobSnapshot` clone per job: label, `current: PathBuf`, prompt, state).
    Today: with a long queue, hundreds of allocations per frame at 60 fps plus lock contention with worker `update`/`sample`.
    Fix: snapshot at most every 100 ms, or only when an engine change counter moved. Effort S. Risk low. Confirmed.

11. **Per-row decoration lookups allocate ~6 times per visible row per frame**
    `crates/soyuz-app/src/ui/tools/decor.rs:56` (`Deco::of`) -> `services/git.rs:687-703` (`key()` = `strip_verbatim` `soyuz-fs/src/path.rs:63-75`: UTF-16 `Vec` + `PathBuf`, twice; `to_string_lossy().replace()`) and `services/tags.rs:81-88` (`norm` = another `strip_verbatim`). Runs for every painted row in every view with `git.status_column = true` (default), also outside any repo.
    Fix: resolve the pane folder's repo root once per frame and pass a borrowed `rel` prefix; make `strip_verbatim` return `Cow` (return the input when there is no `\\?\` prefix). Effort S-M. Risk low. Confirmed.

12. **Cell text (dates, sizes, folder sizes) is formatted from scratch for every visible cell every frame**
    `crates/soyuz-app/src/ui/views/flow.rs:680` -> `views/columns.rs:76-78` -> `soyuz-core/src/time.rs:42-63` (`SystemTime::now()`, chrono `Local` TZ conversion, `format!`) plus `dir_size_text` thread-local borrow per folder row.
    Today: ~50 rows x 2-3 columns of chrono formatting per frame during any animation, then egui hashes the result again for the galley cache.
    Fix: per-pane cell-text cache keyed by (entry idx, column, `vis_version`, minute bucket for relative dates). Effort M. Risk low. Confirmed.

13. **Type-ahead folds every name into a new String per keystroke; `cursor_pos` is O(n) and runs every frame**
    `crates/soyuz-appcore/src/state/pane.rs:544-557` (`fold_for_match(&e.name)` per scanned entry, `locale.rs:280`), `pane.rs:479-482` (`visible_pos` linear scan) used by `cursor_pos()` in `flow.rs:197`, `:503`, `:519` every frame.
    Fix: compare prefixes char-by-char with the existing `fold_char`; keep a `Vec<u32>` entry->visible-position map built in `recompute_visible`. Effort S. Risk low. Confirmed.

14. **Thumbnail eviction holds the slot mutex through three full scans and sorts**
    `crates/soyuz-appcore/src/services/thumbs.rs:379-389` (worker holds `slots` while calling `evict`) -> `:427-482` (count pass, failed pass, byte-sum pass, up to three `by_age` sorts over <= 16k slots, then `publish`); the UI thread's `request()` (`:316`) blocks on the same lock per visible cell.
    Today: ms-scale stalls per decoded image while scrolling a gallery with thousands of cached thumbs, x4 workers.
    Fix: maintain `total_bytes`/`ready`/`failed` incrementally so the common case is O(1); build `by_age` only when over budget. Effort S. Risk low. Confirmed.

15. **Rubber band rebuilds the selection from scratch every frame of the drag**
    `crates/soyuz-app/src/ui/views/flow.rs:456-460`: `items_in_rect` Vec + one `Arc<str>` per hit + `Selection::from_band` clones the base `BTreeSet`, each frame.
    Today: dragging a band across thousands of grid cells allocates thousands of keys per frame.
    Fix: compute hits as index ranges, diff against the previous frame, and apply only the changed keys. Effort M. Risk med. Confirmed.

### Bigger ideas

1. **Precomputed sort keys per entry.** Store a folded, digit-run-encoded name key (and ext/kind id) once at listing time; all sorts become allocation-free `&[u8]` compares (3-5x faster), and watcher patches can binary-search their insertion point instead of a full merge.
2. **Reverse visible index + incremental selection state.** A `Vec<u32>` entry->visible position maintained in `recompute_visible` makes `cursor_pos`, `visible_pos`, `selected_entries` and selection stats O(1)/O(k) and removes most of the O(n) per-click/per-frame paths above in one change.
3. **Row paint cache.** Per pane, cache formatted cell strings, `Deco`, program-icon flag and group ids keyed by `(entry idx, vis_version, theme/i18n generation, width)`; painting becomes lookups, and items 11, 12 and 5 fall out of it.
4. **"Idle means idle" gate.** Audit every periodic thread (theme watcher 1.5 s, watcher liveness 1 s, git 5 s, places 20 s, thumbs trim 15 s) behind a focused/visible flag and add a scripted test (`SOYUZ_SCRIPT` + `SOYUZ_FRAME_LOG`) asserting zero repaints and zero thread wakeups over, say, 30 s of idle.
5. **Listing maintenance off the UI thread.** Let the lister worker own the sorted `Vec<EntryInfo>` + name index + group ids and ship `Arc` snapshots/deltas; the UI thread only swaps a pointer per frame, which removes the streaming-load frame drops (item 2) and makes resort/regroup background work with a stale-but-valid view meanwhile.

## User experience and typography

Scope: `crates/soyuz-app/src/ui`, `crates/soyuz-app/src/theme`, `crates/soyuz-appcore/src/models`. Every finding was verified by reading the code; nothing was compiled or run.

### Findings (ranked by value to a power user who wants configurability, flowing design and impeccable text)

1. **`[appearance] font_size` and the `[fonts.*] size_offset` keys don't reach most of the UI.** `crates/soyuz-app/src/theme/apply.rs:127-143` scales egui's text styles by `k`, but 439 call sites in `crates/soyuz-app/src/ui` hardcode `FontId::proportional(13.0)` etc. (file rows `ui/views/flow.rs:668,688,864`; omnibar `ui/breadcrumb.rs:106,396`; menus `ui/widgets.rs:397,404`; palette `ui/palette/mod.rs:212,369`; tabs `ui/tabstrip.rs:535`; `semibold()` in `flow.rs:56` and a copy in `ui/toasts.rs:27`). Raising the body size enlarges dialog labels and Settings but leaves rows, crumbs, menus and the palette at 13 pt: a mixed-size UI and a setting that half works. Change: add `theme::font(ctx, Role)` / `theme::semibold(ctx, size)` that resolve from `TextStyle::Body`/`Title` (already scaled and offset) and replace the literals mechanically; keep one `semibold()`. Effort L (mechanical). Risk med (row/column constants assume 13 pt; density row heights cover most of it).

2. **End-truncation hides the part of a name that matters.** `ui/widgets.rs:277-281` `trunc_galley` uses `TextWrapping::truncate_at_width` only; the details/list rows (`flow.rs:665-671, 864`), crumbs (`breadcrumb.rs:121,213`, capped at 220 px), pane header path (`ui/pane_view.rs:74-81`), tab titles (`tabstrip.rs:535`) and conflict cards (`ui/dialogs/conflict.rs:368-374`) all lose the extension or the tail (`report-final-v3.pdf` → `report-fin…`). The TUI already keeps the extension (`crates/soyuz-tui/src/view/text.rs:82 truncate_name(keep_ext)`). Change: add `trunc_galley_mid` (binary search on char boundaries over `layout_no_wrap`, cached per text/width) and a `keep_ext` mode; config `[appearance] name_truncation = "keep_extension" | "middle" | "end"` (default keep_extension for file names, middle for paths/crumbs), Settings → Appearance. Effort M. Risk low.

3. **Enter confirms a destructive dialog even when the keyboard focus is on Cancel.** egui clicks the focused button on Enter (`egui-0.37.0/src/context.rs:1761`), and `ui/dialogs/delete_confirm.rs:58`, `ui/dialogs/mod.rs:336` (ConfirmDialog), `:433` and `:522` additionally treat any Enter as the primary. Tab → Cancel → Enter sets both `cancel` and `ok`, and `ok` wins (`delete_confirm.rs:61-66`): a permanent delete from the Cancel button. Change: treat bare Enter as the primary only when `ui.memory(|m| m.focused().is_none())`, as `ui/dialogs/open_with.rs:260` already does, and put that rule inside `footer()`/`close_footer()`. Effort S. Risk low.

4. **The `numbers` font role defaults to proportional figures, so sizes and dates don't line up.** `theme/fonts.rs:358-360` resolves an empty `[fonts.numbers]` to the ui font (Inter, proportional figures by default); egui's `FontTweak` has no OpenType feature switch, so `columns::numeric` (`ui/views/columns.rs:51-57`, used at `flow.rs:685-689`) changes nothing until the user picks a monospace font. The group-header count (`flow.rs:889-893`) and status-bar counts don't use the role at all. Change: embed a feature-frozen tabular subset of Inter (digits, punctuation, ~10 KB; `pyftfeatfreeze -f tnum`) as the `numbers` role's default and first fallback (`Embedded::for_role`), list it as "Inter Tabular" in Settings, and route counts in headers/status bar through `fonts::numbers`. Effort M. Risk low.

5. **Conflict dialog and status bar ignore the user's unit and date settings.** `ui/dialogs/conflict.rs:378` formats with `SizeStyle::Binary`, `:389` with `DateFormatName::Short`; `ui/statusbar.rs:129-133` shows free space in `SizeStyle::Decimal` while the selection size two labels to the left uses `info.size_style` (GiB next to GB). Change: pass `general.size_style`/`date_format` to `PromptModel` (or via `DialogCtx`); free space follows `size_style`, or add `[general] free_space_style` if the decimal-for-disks convention is wanted. Effort S. Risk low.

6. **Two button systems with different heights, colours and feedback.** `ui/dialogs/conflict.rs:39-63` `button()` (28 px, `Color32::WHITE` on accent/danger, no hover animation, used by delete/conflict dialogs) vs `ui/widgets.rs:556-589` `pill_button` (26 px, `p.on_accent()`, used by every other footer), plus a third in `ui/tools/mod.rs:406` and the toast action (`ui/toasts.rs:128-143`). White on a light accent fails the contrast `on_accent()` was written for (`theme/tokens.rs:120`). Change: give `pill_button` the `Btn::{Primary,Secondary,Danger,Ghost}` kinds, animated hover (`animate_bool_with_time`, like `icon_toggle_styled`) and a pressed fill, and delete the other three. Effort M. Risk low.

7. **Theme `[radii] scale` only affects stock egui widgets.** `apply.rs:181` uses `p.radius(...)`; there are zero `p.radius(` calls in `crates/soyuz-app/src/ui`, so rows, menus, pills and cards keep the token values, and several surfaces use ad-hoc radii: welcome card 12 (`ui/welcome.rs:51`), palette 14 (`ui/palette/mod.rs:78`), palette rows 9 (`:336`), search pills 13 (`ui/search.rs:54`), key-sequence chip 15 (`palette/mod.rs:450`). Change: a `p.cr(radius::ROW)` helper used everywhere, plus a `radius::PILL` token for the fully round shapes. Effort M (mechanical). Risk low.

8. **The welcome card has no keyboard exit and never gets out of the way.** `ui/welcome.rs:44-100` is dismissed only by the X / "Got it" buttons or the palette; it sits over the bottom-right of the list for the whole first session. Change: Esc dismisses it when no dialog/palette/Quick Look is open; it auto-dismisses after the user has performed three of its six tips (count in `ui.welcome_seen` state); config `[appearance] welcome_tips = true` (Settings → Appearance) so it can be turned off for new profiles. Effort S. Risk low.

9. **Omnibar completion highlight paints the row text twice.** `ui/breadcrumb.rs:438-448`: `menu_item` paints the name, then the highlighted row gets a translucent `accent_subtle` fill and the plain `name` painted again on top: doubled anti-aliasing (heavier glyphs) and hidden-control characters shown raw instead of as `⟨U+XXXX⟩` tokens. Change: add a `selected` state to `menu_item` (fill + text colour) and drop the overpaint. Effort S. Risk low.

10. **Empty-state hints hardcode key names.** `ui/pane_view.rs:128` (`"Alt+Up"`, `"Alt+Left"`), `:166` (`"Esc"`), `:178` (`"Ctrl+H"`) are literal strings; a rebinding makes them wrong, while `welcome.rs:80` resolves through `chord_label(keymap, action)`. Change: resolve from the keymap in `empty()` (ViewCtx can carry `&Keymap`). Effort S. Risk low.

11. **Empty-state hint rows are centred by a character-count guess.** `ui/empty_state.rs:44` pads by `len*6.5 + 40`, so the kbd + text pair is visibly off-centre for most strings. Change: measure both galleys (`layout_no_wrap`) and the kbd padding, or allocate the row inside `Layout::top_down(Align::Center)`. Effort S. Risk low.

12. **Sub-minimum hit targets in the status bar, unaffected by touch density.** `ui/statusbar.rs:152` theme switch 20×18, `:107` view segments 26×18, `:60` spinner slot; `apply.rs:94-104` enlarges only egui spacing, not these. Change: derive from `ui.spacing().interact_size.y` as `icon_toggle_styled` does (`widgets.rs:188`). Effort S. Risk low.

13. **Open with rows ignore density and show no program icon.** `ui/dialogs/open_with.rs:75` fixes rows at 26 px; the picker draws a generic glyph for every program (`ui/path_picker.rs:471`), although the list view already renders real program icons (`flow.rs:268-275`, `thumbs::get_program_icon`). Change: row height from `interact_size`; give `DialogCtx` (or the picker) a handle to the thumbs service and draw the icon at `ROW_ICON`. Effort M. Risk low.

14. **Path picker: stock egui chips and no help with the extension when saving.** `path_picker.rs:273` uses `egui::Button::selectable` for places (stock look beside the pill buttons); in Save mode (`:204-210`) a name typed without the filter's extension is accepted as is. Change: draw places as pill chips; when exactly one extension is filtered and the typed name has none, append it and show the final name dimmed under the field. Effort S. Risk low.

15. **Rename error bubble: fixed white text and no flip.** `ui/views/rename.rs:102-113` paints `Color32::WHITE` on `p.danger` (wrong for light danger overrides), a 7 pt `▲` from the text font, and always below the row on an unclipped Foreground layer, so for the last row it runs past the window bottom. Change: `on_color(p.danger)` helper (generalize `on_accent`), caret from the icon font, flip above the row when `rect.bottom() + size.y > screen.bottom()`. Effort S. Risk low.

16. **Two reversible settings changes ask first instead of acting with Undo.** `crates/soyuz-appcore/src/models/system_folders.rs:211-218` (hide system folders) and `crates/soyuz-appcore/src/actions.rs:217-224` (Apply view to all) open a ConfirmDialog; both are undoable UI changes. Change: act immediately and push a toast with an Undo action (`Cmd::ShowSystemFolders(true)`; a snapshot of the previous default view for ApplyViewToAll). Effort S / M. Risk low.

17. **Group-header counts and status figures bypass the number font.** `flow.rs:889-893` lays the count out in `FontId::proportional(11.5)`; `statusbar.rs:58-87` likewise. Covered by the fix in 4; listed so it isn't missed. Effort S. Risk low.

18. **PLAN token drift.** `docs/PLAN.md:49` says Monospace 12.5, `theme/tokens.rs:336` has 13.0; shadows in PLAN are 16/32 blur, while welcome (28), dialogs/picker (40), palette (56), toasts (18) each pick their own. Change: add `tokens::shadow::{POPUP, WINDOW, OVERLAY}` and update PLAN §1.2. Effort S. Risk low.

### Ideas

- **Full name on hover for elided rows.** `trunc_galley` already returns `elided` (toasts use it, `toasts.rs:179`); when a row's name galley is elided, show the whole name as a tooltip after ~500 ms, gated by `[appearance] name_tooltips`.
- **Conflict mnemonics and an "Identical" tag.** Underline R / K / S / N on Replace / Keep both / Skip / Rename and accept the bare letters; when `cmp_time` says equal and sizes match (`conflict.rs:155-156`), show an "Identical" tag and make Skip the default answer.
- **Inline ghost completion in the omnibar.** While editing, paint the first completion's remainder in `text_tertiary` after the caret (`breadcrumb.rs:393-400` has the names); Tab already accepts, so the popup becomes optional for touch typists.
- **Match count inside the filter field.** Once a filter is active, replace the Ctrl+F hint at the right of the field (`toolbar.rs:428-437`) with "12 / 340" in the numbers font, so the eye never leaves the toolbar.
- **Type to filter in Open with.** When the typed text in the program field is not a path, filter the registered/remembered list live and keep ↑/↓ on the filtered rows (the picker already has this pattern, `path_picker.rs:372-389`).
- **Modifier clicks on crumbs.** Shift+click opens the crumb in the other pane (`Cmd::OpenInOtherPane` exists), Ctrl+click copies its path; both are one `resp.clicked()` branch in `breadcrumb.rs:222-231`.
- **Esc dismisses the newest toast.** With nothing else open, Esc closes the top toast (and Shift+Esc all), giving keyboard users the X button they can't reach (`toasts.rs:150-168`).
- **Welcome as a status-bar strip.** Instead of the card, cycle the six tips as one dim line at the left of the status bar for the first sessions (`statusbar.rs:56-89`); it occupies no list space and vanishes once the tips have been used.
