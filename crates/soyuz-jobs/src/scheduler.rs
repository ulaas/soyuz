//! Admission rules (PLAN §2.4), implemented in [`crate::engine`] (`Inner::admit`,
//! `Inner::compute_lanes`) because they operate directly on the engine state:
//!
//! * a queued job starts when `active < max_parallel` (active = `Planning | Running |
//!   Finishing`; `Paused`/`WaitingForUser` do not count), its lane is free
//!   (`PerDevice` → `(src_dev, dst_dev)` of its first/last touched root, `Serial` → one
//!   global lane, `Parallel` → no lane; only *active* jobs occupy a lane), and none of its
//!   `touched_roots()` overlaps (`soyuz_fs::path_overlaps`) a root of any started,
//!   unfinished job (paused and waiting jobs included) or of a job ahead of it in the queue
//!   that could not start in this pass (keeps submission order on the same tree);
//! * queue order = `(Priority desc, submission asc)`, changed by `set_priority`,
//!   `move_in_queue` and `start_now` (front of the queue, bypasses lane and
//!   `max_parallel` once; root overlap is still enforced);
//! * `JobOptions::queue == PerDevice` (the default) defers to `EngineConfig::default_queue`.
