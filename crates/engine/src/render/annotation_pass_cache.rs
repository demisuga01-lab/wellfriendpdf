//! Document objects shared by every annotation of one annotation pass.
//!
//! PDFium parses a document's fonts and form dictionary once
//! (`CPDF_DocPageData` font cache, `CPDF_InteractiveForm`) and every widget
//! of a page draws through them. Wellfriend's widget appearance synthesis
//! (`/NeedAppearances`, missing `/AP`) used to resolve the catalog's
//! `/AcroForm` dictionary — cloning its `/Fields` array — and rebuild the
//! `/DR` field font (loading the font program and decoding all 256 codes into
//! an encoding and width table) for every widget. On a tax form with a few
//! hundred fields that was most of the annotation pass.
//!
//! [`AnnotationPassCacheGuard`] installs a per-thread cache for the duration
//! of one annotation pass (it is dropped at the end of the pass, so nothing
//! is shared across documents or revisions). Outside a pass the lookups just
//! compute their value.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::object::PdfDictionary;

/// What a cached value is keyed by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PassCacheKey {
    /// Layout metrics of the simple font at this indirect object.
    SimpleFont(u32, u16),
    /// Layout metrics of a standard-14 font by base name.
    StandardFont(String),
    /// Object references of every node of the AcroForm `/Fields` tree.
    FormControls,
}

#[derive(Default)]
struct PassCache {
    acroform: Option<Option<Arc<PdfDictionary>>>,
    values: HashMap<PassCacheKey, Arc<dyn Any + Send + Sync>>,
}

thread_local! {
    static PASS_CACHE: RefCell<Option<PassCache>> = const { RefCell::new(None) };
}

/// Scoped cache for one annotation pass on the current thread. Nested passes
/// get their own cache and restore the outer one when they end.
pub(crate) struct AnnotationPassCacheGuard {
    previous: Option<PassCache>,
}

impl AnnotationPassCacheGuard {
    pub(crate) fn enter() -> Self {
        let previous = PASS_CACHE.with(|cell| cell.replace(Some(PassCache::default())));
        Self { previous }
    }
}

impl Drop for AnnotationPassCacheGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        PASS_CACHE.with(|cell| *cell.borrow_mut() = previous);
    }
}

/// The document's `/AcroForm` dictionary, resolved once per pass.
pub(crate) fn acroform(
    resolve: impl FnOnce() -> Option<PdfDictionary>,
) -> Option<Arc<PdfDictionary>> {
    let cached = PASS_CACHE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|cache| cache.acroform.clone())
    });
    if let Some(value) = cached {
        return value;
    }
    let value = resolve().map(Arc::new);
    PASS_CACHE.with(|cell| {
        if let Some(cache) = cell.borrow_mut().as_mut() {
            cache.acroform = Some(value.clone());
        }
    });
    value
}

/// The value for `key`, built once per pass.
pub(crate) fn shared<T: Any + Send + Sync>(key: PassCacheKey, build: impl FnOnce() -> T) -> Arc<T> {
    let cached = PASS_CACHE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|cache| cache.values.get(&key).cloned())
    });
    if let Some(value) = cached.and_then(|value| value.downcast::<T>().ok()) {
        return value;
    }
    let value = Arc::new(build());
    PASS_CACHE.with(|cell| {
        if let Some(cache) = cell.borrow_mut().as_mut() {
            cache
                .values
                .insert(key, Arc::clone(&value) as Arc<dyn Any + Send + Sync>);
        }
    });
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_shared_only_inside_a_pass() {
        let builds = std::cell::Cell::new(0);
        let build = || {
            builds.set(builds.get() + 1);
            7u32
        };
        assert_eq!(*shared(PassCacheKey::SimpleFont(1, 0), build), 7);
        assert_eq!(*shared(PassCacheKey::SimpleFont(1, 0), build), 7);
        assert_eq!(builds.get(), 2, "no caching outside a pass");
        {
            let _pass = AnnotationPassCacheGuard::enter();
            shared(PassCacheKey::SimpleFont(1, 0), build);
            shared(PassCacheKey::SimpleFont(1, 0), build);
            shared(PassCacheKey::SimpleFont(2, 0), build);
            assert_eq!(builds.get(), 4);
            let mut resolves = 0;
            for _ in 0..3 {
                assert!(acroform(|| {
                    resolves += 1;
                    None
                })
                .is_none());
            }
            assert_eq!(resolves, 1);
        }
        shared(PassCacheKey::SimpleFont(1, 0), build);
        assert_eq!(builds.get(), 5, "the cache ends with the pass");
    }
}
