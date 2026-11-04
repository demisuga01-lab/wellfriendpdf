//! Measures which image placements and path groups the editor can select and
//! edit: before (the `wellfriendpdf-edit` recognisers) and after (every
//! object `content_objects` lists, each moved and deleted to verify it).
//!
//! `cargo run --release -p wellfriendpdf-editor --example object_coverage -- [--pages N] [--sample N] FILE...`
//! Files over 64 MB are measured on one-page windows of their first pages.

use std::collections::BTreeMap;

use wellfriend_pdf_edit::PdfMatrix;
use wellfriendpdf_editor::content_objects::{page_content_objects, ContentObject, ContentObjectKind, ObjectEdit};
use wellfriendpdf_editor::{apply_operation, page_objects, EditorOperation, MemoryFonts};

#[derive(Default, Clone, Copy)]
struct Tally {
    total: usize,
    before: usize,
    listed: usize,
    sampled: usize,
    moved: usize,
    deleted: usize,
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.total += other.total;
        self.before += other.before;
        self.listed += other.listed;
        self.sampled += other.sampled;
        self.moved += other.moved;
        self.deleted += other.deleted;
    }
}

fn category(object: &ContentObject) -> &'static str {
    let image = matches!(object.selector.kind, ContentObjectKind::Image | ContentObjectKind::InlineImage);
    let order: [(&str, &str); 9] = [
        ("inlineImage", "inline image"),
        ("inForm", "in a Form XObject"),
        ("shading", "shading (sh)"),
        ("softMask", "soft mask / mask"),
        ("stencilMask", "stencil mask"),
        ("pattern", "pattern fill/stroke"),
        ("clipAndPaint", "clip + paint path"),
        ("clipped", "under a clip path"),
        ("nestedState", "nested q/Q"),
    ];
    for (trait_name, label) in order {
        if object.has(trait_name) {
            return label;
        }
    }
    if object.has("graphicsState") {
        return "ExtGState (gs) set";
    }
    if image {
        "plain image"
    } else {
        "plain path"
    }
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.75)
}

fn verify(bytes: &[u8], page: u32, object: &ContentObject, count: usize, fonts: &MemoryFonts) -> (bool, bool, Option<String>) {
    let (dx, dy) = (7.0, 5.0);
    let moved_op = EditorOperation::EditObject {
        selector: object.selector.clone(),
        edit: ObjectEdit::Transform { matrix: PdfMatrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: dx, f: dy } },
    };
    let mut error = None;
    let moved = match apply_operation(bytes, &moved_op, fonts, true) {
        Ok((out, _)) => {
            let document = lopdf::Document::load_mem(&out).unwrap();
            let after = page_content_objects(&document, page).unwrap();
            let target = [object.bounds[0] + dx, object.bounds[1] + dy, object.bounds[2] + dx, object.bounds[3] + dy];
            let ok = after.objects.len() == count && after.objects.iter().any(|o| o.selector.kind == object.selector.kind && near(o.bounds, target));
            if !ok {
                error = Some(format!("moved but not found at target ({} objects, was {count})", after.objects.len()));
            }
            ok
        }
        Err(e) => {
            error = Some(format!("move: {} {}", e.code, e.message));
            false
        }
    };
    let delete_op = EditorOperation::EditObject { selector: object.selector.clone(), edit: ObjectEdit::Delete };
    let deleted = match apply_operation(bytes, &delete_op, fonts, true) {
        Ok((out, _)) => {
            let document = lopdf::Document::load_mem(&out).unwrap();
            let after = page_content_objects(&document, page).unwrap();
            let ok = after.objects.len() + 1 == count;
            if !ok && error.is_none() {
                error = Some(format!("deleted: {} objects, was {count}", after.objects.len()));
            }
            ok
        }
        Err(e) => {
            error.get_or_insert(format!("delete: {} {}", e.code, e.message));
            false
        }
    };
    (moved, deleted, error)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut pages = 3usize;
    let mut sample = 40usize;
    while args.first().is_some_and(|a| a.starts_with("--")) {
        let flag = args.remove(0);
        let value: usize = args.remove(0).parse().unwrap();
        match flag.as_str() {
            "--pages" => pages = value,
            "--sample" => sample = value,
            _ => panic!("unknown flag {flag}"),
        }
    }
    let fonts = MemoryFonts::new();
    let mut by_category: BTreeMap<(&'static str, &'static str), Tally> = BTreeMap::new();
    let mut failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut measured_pages = 0usize;
    let mut unreadable_pages = 0usize;
    for path in &args {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        // (bytes, page in bytes) per measured page
        let mut units: Vec<(Vec<u8>, u32)> = Vec::new();
        if size > 64 * 1024 * 1024 {
            let engine = wellfriendpdf_engine::ContentEngine::open_path(path).unwrap();
            let count = engine.page_count().unwrap();
            for page in 1..=count.min(pages) {
                units.push((wellfriendpdf_editor::windows::extract(&engine, page as u32).unwrap(), 1));
            }
        } else {
            let bytes = std::fs::read(path).unwrap();
            let Ok(document) = lopdf::Document::load_mem(&bytes) else {
                eprintln!("{path}: lopdf cannot read it");
                continue;
            };
            let count = document.get_pages().len();
            for page in 1..=count.min(pages) {
                units.push((bytes.clone(), page as u32));
            }
        }
        let mut file = Tally::default();
        for (bytes, page) in units {
            let Ok(document) = lopdf::Document::load_mem(&bytes) else { continue };
            let Ok(all) = page_content_objects(&document, page) else { continue };
            measured_pages += 1;
            if all.content_unreadable {
                eprintln!("UNREADABLE {path} p{page}");
                unreadable_pages += 1;
            }
            let listed = page_objects(&bytes, page);
            let Ok(listed) = listed else {
                eprintln!("{path} p{page}: page_objects failed: {:?}", listed.err());
                continue;
            };
            let count = all.objects.len();
            // Objects `objects` lists are the ones the legacy lists miss.
            let uncovered: Vec<_> = listed.objects.iter().map(|o| o.selector.clone()).collect();
            let step = (count / sample.max(1)).max(1);
            for (index, object) in all.objects.iter().enumerate() {
                let kind = match object.selector.kind {
                    ContentObjectKind::Image | ContentObjectKind::InlineImage => "image",
                    _ => "path",
                };
                let legacy = !uncovered.contains(&object.selector);
                let mut tally = Tally { total: 1, before: usize::from(legacy), listed: 1, ..Tally::default() };
                if index % step == 0 {
                    tally.sampled = 1;
                    let (moved, deleted, error) = verify(&bytes, page, object, count, &fonts);
                    tally.moved = usize::from(moved);
                    tally.deleted = usize::from(deleted);
                    if let Some(error) = error {
                        eprintln!("FAIL {path} p{page} {:?} {}..{} forms {}: {error}", object.selector.kind, object.selector.start, object.selector.end, object.selector.forms.len());
                        *failures.entry(format!("{kind}/{}: {error}", category(object))).or_default() += 1;
                    }
                }
                by_category.entry((kind, category(object))).or_default().add(tally);
                file.add(tally);
            }
        }
        println!(
            "{:<60} objects {:>6}  selectable before {:>6} ({:>5.1}%)  after {:>6}",
            path.rsplit('/').next().unwrap_or(path),
            file.total,
            file.before,
            100.0 * file.before as f64 / file.total.max(1) as f64,
            file.listed
        );
    }
    println!("\npages measured {measured_pages}, content unreadable {unreadable_pages}\n");
    println!("{:<6} {:<22} {:>7} {:>8} {:>8} {:>8} {:>9} {:>9}", "kind", "category", "objects", "before", "before%", "after%", "sampled", "move+del ok");
    let mut totals: BTreeMap<&str, Tally> = BTreeMap::new();
    for ((kind, _), tally) in &by_category {
        totals.entry(kind).or_default().add(*tally);
    }
    for ((kind, category), tally) in &by_category {
        let sampled = tally.sampled;
        println!(
            "{:<6} {:<22} {:>7} {:>8} {:>7.1}% {:>7.1}% {:>9} {:>4}/{:<4}",
            kind,
            category,
            tally.total,
            tally.before,
            100.0 * tally.before as f64 / tally.total.max(1) as f64,
            100.0 * tally.listed as f64 / tally.total.max(1) as f64,
            sampled,
            tally.moved,
            tally.deleted
        );
    }
    for (kind, tally) in &totals {
        let sampled = tally.sampled;
        println!(
            "{:<6} {:<22} {:>7} {:>8} {:>7.1}% {:>7.1}%   sampled {sampled}: moved {} deleted {}",
            kind,
            "ALL",
            tally.total,
            tally.before,
            100.0 * tally.before as f64 / tally.total.max(1) as f64,
            100.0 * tally.listed as f64 / tally.total.max(1) as f64,
            tally.moved,
            tally.deleted
        );
    }
    if !failures.is_empty() {
        println!("\nfailures:");
        for (failure, count) in failures {
            println!("  {count:>5}  {failure}");
        }
    }
}
