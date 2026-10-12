//! Manual performance probe for large listings (`cargo test -p soyuz-appcore --test
//! perf_probe -- --ignored --nocapture`).

use std::time::Instant;

use soyuz_appcore::state::pane::Pane;
use soyuz_appcore::state::view_memory::ViewState;
use soyuz_core::{EntryInfo, EntryKind, Location};

#[test]
#[ignore]
fn probe_100k() {
    let mut p = Pane::new(Location::Dir("/big".into()), ViewState::default());
    let t = Instant::now();
    let all: Vec<EntryInfo> =
        (0..100_000).map(|i| EntryInfo::minimal(format!("/big/file_{i}.dat").into(), EntryKind::File)).collect();
    let mut worst = 0f64;
    for (n, chunk) in all.chunks(512).enumerate() {
        let done = (n + 1) * 512 >= all.len();
        let f = Instant::now();
        p.on_listing(chunk.to_vec(), done, None);
        if n % 4 == 0 || done {
            p.refresh_visible(true, true);
        }
        worst = worst.max(f.elapsed().as_secs_f64());
    }
    println!("total {:?} worst step {:.3}s", t.elapsed(), worst);
    assert_eq!(p.visible.len(), 100_000);
}
