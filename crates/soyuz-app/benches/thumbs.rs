//! Thumbnail pipeline cost: decode + downscale, mirroring `services::thumbs::decode`
//! (`cargo bench -p soyuz-app --bench thumbs`). Inputs: a 4000×3000 JPEG and a 2048×2048
//! PNG generated into a temp folder.

use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

/// Same steps and limits as the app's thumbnail worker.
fn decode_thumb(path: &Path, px: u32) -> image::RgbaImage {
    let mut reader = image::ImageReader::open(path).expect("open").with_guessed_format().expect("format");
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().expect("decoder");
    let orientation = image::ImageDecoder::orientation(&mut decoder).ok();
    let mut img = image::DynamicImage::from_decoder(decoder).expect("decode");
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    img.thumbnail(px, px).to_rgba8()
}

fn gradient(w: u32, h: u32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8]))
}

fn thumbs(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("tempdir");
    let jpg = dir.path().join("photo.jpg");
    gradient(4000, 3000).save(&jpg).expect("save jpg");
    let png = dir.path().join("art.png");
    gradient(2048, 2048).save(&png).expect("save png");

    let mut g = c.benchmark_group("thumbs");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(10));
    g.bench_function("jpeg_12mp_to_256", |b| b.iter(|| black_box(decode_thumb(&jpg, 256))));
    g.bench_function("png_4mp_to_256", |b| b.iter(|| black_box(decode_thumb(&png, 256))));
    g.finish();
}

criterion_group!(benches, thumbs);
criterion_main!(benches);
