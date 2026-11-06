//! Performance counters (pdh.dll) under their English names, so any display language works.

use std::ptr::null;

use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};

use super::super::wide_str;

/// Do not cap percentages at 100 (`PDH_FMT_NOCAP100`).
const FMT_NOCAP100: u32 = 0x0000_8000;
/// The largest formatted array accepted.
const MAX_ARRAY_BYTES: usize = 64 << 20;

/// One query with its counters. Rate counters need two collections before they give data.
pub(super) struct Query {
    handle: PDH_HQUERY,
    counters: Vec<PDH_HCOUNTER>,
    scratch: Vec<u64>,
}

// SAFETY: pdh query and counter handles are process-wide and not tied to a thread; `Query`
// is used by one thread at a time because every method takes `&mut self`.
unsafe impl Send for Query {}

impl Query {
    pub(super) fn new() -> Option<Query> {
        let mut handle: PDH_HQUERY = std::ptr::null_mut();
        // SAFETY: null data source means live data; `handle` is a valid out-pointer.
        let rc = unsafe { PdhOpenQueryW(null(), 0, &mut handle) };
        (rc == 0).then(|| Query { handle, counters: Vec::new(), scratch: Vec::new() })
    }

    /// A query with the given counters; `None` when one of them does not exist here.
    pub(super) fn with(paths: &[&str]) -> Option<Query> {
        let mut q = Query::new()?;
        for p in paths {
            q.add(p)?;
        }
        Some(q)
    }

    /// Add a counter, returning its index for [`Self::values`].
    fn add(&mut self, path: &str) -> Option<usize> {
        let w = wide_str(path);
        let mut c: PDH_HCOUNTER = std::ptr::null_mut();
        // SAFETY: open query, NUL-terminated counter path, valid out-pointer.
        let rc = unsafe { PdhAddEnglishCounterW(self.handle, w.as_ptr(), 0, &mut c) };
        if rc != 0 {
            return None;
        }
        self.counters.push(c);
        Some(self.counters.len() - 1)
    }

    /// Take a new reading of every counter.
    pub(super) fn collect(&mut self) -> bool {
        // SAFETY: open query.
        unsafe { PdhCollectQueryData(self.handle) == 0 }
    }

    /// Call `f(instance, value)` for each valid instance of counter `index`. Nothing is
    /// called when there is no data yet.
    pub(super) fn values(&mut self, index: usize, mut f: impl FnMut(&[u16], f64)) {
        let Some(&counter) = self.counters.get(index) else { return };
        let fmt = PDH_FMT_DOUBLE | FMT_NOCAP100;
        let item_size = size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        for _ in 0..4 {
            let mut bytes = (self.scratch.len() * 8).min(u32::MAX as usize) as u32;
            let mut count = 0u32;
            // SAFETY: `scratch` is writable for `bytes` bytes and 8-aligned, the alignment of
            // the items; `bytes` and `count` are valid out-pointers.
            let rc = unsafe {
                PdhGetFormattedCounterArrayW(
                    counter,
                    fmt,
                    &mut bytes,
                    &mut count,
                    self.scratch.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                )
            };
            if rc == PDH_MORE_DATA {
                let want = bytes as usize + (4 << 10);
                if want > MAX_ARRAY_BYTES {
                    return;
                }
                self.scratch.resize(want.div_ceil(8), 0);
                continue;
            }
            if rc != 0 {
                return;
            }
            let total = self.scratch.len() * 8;
            let n = (count as usize).min(total / item_size);
            let base = self.scratch.as_ptr().cast::<u8>();
            for i in 0..n {
                // SAFETY: `i < n` items fit in the buffer, which holds what the call wrote.
                let item = unsafe { std::ptr::read(base.add(i * item_size).cast::<PDH_FMT_COUNTERVALUE_ITEM_W>()) };
                let st = item.FmtValue.CStatus;
                if st != PDH_CSTATUS_VALID_DATA && st != PDH_CSTATUS_NEW_DATA {
                    continue;
                }
                // SAFETY: PDH_FMT_DOUBLE selects the double member of the union.
                let value = unsafe { item.FmtValue.Anonymous.doubleValue };
                let name = instance_name(base as usize, total, item.szName as usize);
                if value.is_finite() {
                    f(name, value);
                }
            }
            return;
        }
    }
}

/// The NUL-terminated instance name at `ptr`, which pdh stores in the caller's buffer.
fn instance_name<'a>(base: usize, total: usize, ptr: usize) -> &'a [u16] {
    let Some(off) = ptr.checked_sub(base) else { return &[] };
    if off >= total || !ptr.is_multiple_of(2) {
        return &[];
    }
    let room = (total - off) / 2;
    // SAFETY: `ptr` lies inside the live buffer with `room` u16 after it and is aligned; the
    // name is only used while the caller's `values` loop runs, before the buffer changes.
    let units = unsafe { std::slice::from_raw_parts(ptr as *const u16, room) };
    let end = units.iter().position(|&c| c == 0).unwrap_or(room);
    &units[..end]
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: open query closed once; this also removes its counters.
        unsafe {
            PdhCloseQuery(self.handle);
        }
    }
}
