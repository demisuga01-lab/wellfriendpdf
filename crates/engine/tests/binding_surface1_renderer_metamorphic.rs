use std::path::PathBuf;

use wellfriendpdf_engine::{
    CancelToken, ContentEngine, PixelBuffer, RenderCache, RenderMode, RenderTile, WHITE,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn simple_pdf(content: &str) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
    ];
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (idx, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let startxref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            startxref
        )
        .as_bytes(),
    );
    out
}

fn assert_same_pixels(expected: &PixelBuffer, actual: &PixelBuffer) {
    assert_eq!(expected.width, actual.width);
    assert_eq!(expected.height, actual.height);
    assert_eq!(expected.to_rgba_bytes(), actual.to_rgba_bytes());
}

fn stitch_tiles(engine: &ContentEngine, page: usize, dpi: u32, tile_size: u32) -> PixelBuffer {
    let full = engine
        .render_page_with_mode(page, dpi, RenderMode::Compat)
        .expect("full render for dimensions");
    let mut stitched =
        PixelBuffer::new_filled_with_mode(full.width, full.height, WHITE, RenderMode::Compat);
    let mut y = 0;
    while y < full.height {
        let height = tile_size.min(full.height - y);
        let mut x = 0;
        while x < full.width {
            let width = tile_size.min(full.width - x);
            let tile = RenderTile {
                x,
                y,
                width,
                height,
            };
            let piece = engine
                .render_page_tile_with_mode(page, dpi, tile, RenderMode::Compat, None)
                .expect("tile render");
            for yy in 0..height {
                for xx in 0..width {
                    stitched.set_pixel(
                        (x + xx) as i32,
                        (y + yy) as i32,
                        piece.get_pixel(xx as i32, yy as i32),
                    );
                }
            }
            x += width;
        }
        y += height;
    }
    stitched
}

fn stitch_bands(engine: &ContentEngine, page: usize, dpi: u32, band_height: u32) -> PixelBuffer {
    let full = engine
        .render_page_with_mode(page, dpi, RenderMode::Compat)
        .expect("full render for dimensions");
    let bands = engine
        .render_page_bands_with_mode(page, dpi, band_height, RenderMode::Compat)
        .expect("band render");
    let mut stitched =
        PixelBuffer::new_filled_with_mode(full.width, full.height, WHITE, RenderMode::Compat);
    let mut y_offset = 0;
    for band in bands {
        for y in 0..band.height {
            for x in 0..band.width {
                stitched.set_pixel(
                    x as i32,
                    (y_offset + y) as i32,
                    band.get_pixel(x as i32, y as i32),
                );
            }
        }
        y_offset += band.height;
    }
    stitched
}

#[test]
fn renderer_fuzz_cmm_full_tile_band_and_tile_size_equivalence() {
    let cases = [
        ContentEngine::open_bytes(simple_pdf(
            "1 0 0 rg 0 0 50 100 re f\n0 0 1 rg 50 0 50 100 re f\n",
        ))
        .expect("open vector pdf"),
        ContentEngine::open_path(fixture("flate.pdf")).expect("open text fixture"),
        ContentEngine::open_path(fixture("image_only.pdf")).expect("open image fixture"),
        ContentEngine::open_path(fixture("attach_annot.pdf")).expect("open annotation fixture"),
    ];

    for engine in cases {
        let full = engine
            .render_page_with_mode(1, 36, RenderMode::Compat)
            .expect("full render");
        assert_same_pixels(&full, &stitch_tiles(&engine, 1, 36, 37));
        assert_same_pixels(&full, &stitch_tiles(&engine, 1, 36, 79));
        assert_same_pixels(&full, &stitch_bands(&engine, 1, 36, 23));
        assert_same_pixels(&full, &stitch_bands(&engine, 1, 36, 61));
    }
}

#[test]
fn renderer_fuzz_cmm_cache_cold_warm_and_no_cache_are_equivalent() {
    let engine = ContentEngine::open_bytes(simple_pdf("1 0 0 rg 0 0 100 100 re f\n"))
        .expect("open vector pdf");
    let tile = RenderTile {
        x: 0,
        y: 0,
        width: 50,
        height: 50,
    };
    let no_cache = engine
        .render_page_tile_with_mode(1, 72, tile, RenderMode::Compat, None)
        .expect("tile no cache");
    let mut cache = RenderCache::new(20_000, 20_000);
    let cold = engine
        .render_page_tile_with_mode(1, 72, tile, RenderMode::Compat, Some(&mut cache))
        .expect("tile cold cache");
    let warm = engine
        .render_page_tile_with_mode(1, 72, tile, RenderMode::Compat, Some(&mut cache))
        .expect("tile warm cache");
    assert_same_pixels(&no_cache, &cold);
    assert_same_pixels(&cold, &warm);
    assert_eq!(cache.metrics().hits, 1);
}

#[test]
fn renderer_fuzz_cmm_progressive_resume_matches_uninterrupted_render() {
    let engine = ContentEngine::open_bytes(simple_pdf(
        "q 1 0 0 rg 10 10 40 40 re f Q\nq 0 0 1 rg 50 50 40 40 re f Q\n",
    ))
    .expect("open vector pdf");
    let full = engine
        .render_page_with_mode(1, 72, RenderMode::Compat)
        .expect("full render");
    let mut job = engine
        .progressive_render_job_with_mode(1, 72, 25, 25, RenderMode::Compat)
        .expect("create progressive job");
    job.render_next(3, &CancelToken::none())
        .expect("first progressive step");
    job.validate_resume_token(&job.token())
        .expect("resume token validates");
    while !job.is_complete() {
        job.render_next(2, &CancelToken::none())
            .expect("progressive resume step");
    }
    let progressive = job.finish().expect("completed progressive render");
    assert_same_pixels(&full, &progressive);
}

#[test]
fn renderer_fuzz_cmm_cancelled_render_does_not_poison_later_render() {
    let engine = ContentEngine::open_bytes(simple_pdf("1 0 0 rg 0 0 100 100 re f\n"))
        .expect("open vector pdf");
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(engine
        .render_page_cancellable_with_mode(1, 72, &cancel, RenderMode::Compat)
        .is_err());
    let after_cancel = engine
        .render_page_with_mode(1, 72, RenderMode::Compat)
        .expect("render after cancellation");
    let repeated = engine
        .render_page_with_mode(1, 72, RenderMode::Compat)
        .expect("repeat render");
    assert_same_pixels(&after_cancel, &repeated);
}
