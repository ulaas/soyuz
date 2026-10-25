//! Memory per listed entry: lists a temp folder of 20 000 files the way a pane does
//! (streamed chunks merged into `Pane`) and prints the bytes held per entry.
//! `cargo test -p soyuz-appcore --release --test listing_memory -- --ignored --nocapture`

use soyuz_appcore::state::pane::Pane;
use soyuz_appcore::state::view_memory::ViewState;
use soyuz_core::{Control, EntryInfo, Location, OpControl};

const N: usize = 20_000;

#[test]
#[ignore = "perf probe: creates 20 000 files"]
fn bytes_per_entry() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..N {
        std::fs::write(dir.path().join(format!("IMG_{i:05}_holiday-photo.jpg")), b"").unwrap();
    }
    let before = soyuz_fs::process::memory().expect("memory counters");
    let mut pane = Pane::new(Location::Dir(dir.path().to_path_buf()), ViewState::default());
    let ctl = OpControl::new();
    // Merged one chunk per "frame", as `Pane::flush_incoming` does while streaming.
    soyuz_fs::read_dir_streaming(dir.path(), &soyuz_fs::ListOptions::default(), &ctl, &mut |c| {
        pane.on_listing(c, false, None);
        pane.flush_incoming();
        Control::Continue
    })
    .unwrap();
    pane.on_listing(Vec::new(), true, None);
    pane.refresh_visible(true, true);
    let after = soyuz_fs::process::memory().expect("memory counters");
    assert_eq!(pane.entries.len(), N);

    let heap: usize = pane.entries.iter().map(entry_heap).sum();
    let delta = after.private.saturating_sub(before.private);
    println!(
        "size_of::<EntryInfo>() = {} B; entries capacity {} (len {N}); own heap per entry {} B",
        std::mem::size_of::<EntryInfo>(),
        pane.entries.capacity(),
        heap / N,
    );
    println!("private bytes: +{} KB for {N} entries = {} B per entry", delta / 1024, delta as usize / N,);
    drop(pane);
}

/// Heap bytes an entry owns (string buffers), without allocator overhead.
fn entry_heap(e: &EntryInfo) -> usize {
    e.path.capacity() + e.name.capacity()
}
