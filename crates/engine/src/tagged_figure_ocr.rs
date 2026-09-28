//! Whole-Figure OCR capture. By default, every selected carrier must belong
//! exclusively to the existing approved image's Figure leaf. A caller may
//! instead partition exact spans across selected, content-only P/Span siblings
//! and approve their removal through `merge_into_figure`; semantic subtrees are
//! never inferred or flattened implicitly.
use super::*;
use crate::advanced_editing::MultiRunRangeModel;

pub(in crate::tagged_structure::story) struct OcrSelection {
    expected: BTreeMap<(usize, String), (String, ObjectRef)>,
    seen: BTreeSet<(usize, String)>,
}
#[derive(Default)]
pub(in crate::tagged_structure::story) struct PageOcr {
    operands: BTreeMap<(usize, ObjectRef, [usize; 2]), ObjectRef>,
    actual: BTreeMap<String, BTreeSet<usize>>,
    owners: BTreeMap<String, BTreeSet<ObjectRef>>,
}
impl PageOcr {
    pub(super) fn carries_actual_text(&self, figure: &str, mark: usize) -> bool {
        self.actual.get(figure).is_some_and(|m| m.contains(&mark))
    }
    pub(in crate::tagged_structure::story) fn contains(&self, occurrence: &TextOccurrence) -> bool {
        self.operands
            .contains_key(&(occurrence.stream_index, occurrence.stream, occurrence.range))
    }
    pub(super) fn owner_of(&self, occurrence: &TextOccurrence) -> Option<ObjectRef> {
        self.operands
            .get(&(occurrence.stream_index, occurrence.stream, occurrence.range))
            .copied()
    }
    pub(super) fn owners(&self, figure: &str) -> BTreeSet<ObjectRef> {
        self.owners.get(figure).cloned().unwrap_or_default()
    }
}
impl OcrSelection {
    pub(in crate::tagged_structure::story) fn new(
        request: &LinkedStoryRequest,
        selection: &Selection,
    ) -> Result<Self> {
        let mut expected = BTreeMap::new();
        for figure in &request.figures {
            let Some(ocr) = &figure.ocr else {
                continue;
            };
            let ImageFragmentSource::Occurrence { page, .. } = &figure.source else {
                return Err(fail("owned tagged groups already define their OCR scope"));
            };
            let figure_owner = selection
                .figures
                .get(&figure.id)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    fail("tagged OCR capture requires its existing selected Figure owner")
                })?;
            if ocr.span_ids.is_empty()
                || ocr.span_ids.len() > 4096
                || ocr.expected_text.len() > 4_000_000
                || ocr.form_target.as_ref().is_some_and(|target| {
                    target.page != *page || target.input_sha256 != request.input_sha256
                })
            {
                return Err(fail("invalid tagged OCR selection budget"));
            }
            let identity_limit = if ocr.form_target.is_some() { 512 } else { 128 };
            for span in &ocr.span_ids {
                let owner = selection
                    .figure_ocr_owners
                    .get(&figure.id)
                    .and_then(|owners| owners.get(span))
                    .copied()
                    .unwrap_or(figure_owner);
                if span.is_empty()
                    || span.len() > identity_limit
                    || expected.len() >= 65_536
                    || expected
                        .insert((*page, span.clone()), (figure.id.clone(), owner))
                        .is_some()
                {
                    return Err(fail("duplicate/invalid tagged OCR carrier selection"));
                }
            }
        }
        Ok(Self {
            expected,
            seen: BTreeSet::new(),
        })
    }

    pub(in crate::tagged_structure::story) fn page(
        &mut self,
        page: usize,
        page_id: ObjectRef,
        marks: &PageMarks,
        index: &StructureIndex,
        model: &MultiRunRangeModel,
        request: &LinkedStoryRequest,
    ) -> Result<PageOcr> {
        let mut bound = PageOcr::default();
        if self
            .expected
            .range((page, String::new())..)
            .next()
            .is_none_or(|((source_page, _), _)| *source_page != page)
        {
            return Ok(bound);
        }
        let spans = model
            .source_spans
            .iter()
            .map(|s| (s.span_id.as_str(), s))
            .collect::<BTreeMap<_, _>>();
        for occurrence in &marks.text {
            crate::cancel::check_current_cancel("tagged Figure OCR ownership")?;
            let span_id = format!(
                "p{page}:s{}:o{}",
                occurrence.stream_index, occurrence.range[0]
            );
            let identity = (page, span_id.clone());
            let Some((figure, owner)) = self.expected.get(&identity) else {
                continue;
            };
            let span = spans
                .get(span_id.as_str())
                .ok_or_else(|| fail("tagged OCR source operand is missing"))?;
            if span.text_render_mode != 3
                || occurrence.bytes_empty
                || (span.stream_object, span.stream_generation) != occurrence.stream
                || span.byte_range != occurrence.range
            {
                return Err(fail(
                    "tagged OCR selection is not its exact invisible source operand",
                ));
            }
            let owners = occurrence
                .marks
                .iter()
                .filter_map(|i| marks.marks[*i].mcid)
                .map(|m| {
                    index
                        .marked
                        .get(&page_id)
                        .and_then(|v| v.get(&m))
                        .copied()
                        .ok_or_else(|| fail("tagged OCR has unresolved MCID ownership"))
                })
                .collect::<Result<BTreeSet<_>>>()?;
            if owners != BTreeSet::from([*owner]) {
                return Err(fail(
                    "OCR belongs to a different or nested semantic owner; whole-Figure capture cannot flatten it",
                ));
            }
            for &i in &occurrence.marks {
                let mark = &marks.marks[i];
                if !mark.safe || mark.artifact || mark.frame.is_some() || mark.image_key.is_some() {
                    return Err(fail(
                        "tagged OCR has unapproved optional/artifact/frame ownership",
                    ));
                }
                if mark.actual_text {
                    bound.actual.entry(figure.clone()).or_default().insert(i);
                }
            }
            if request
                .frames
                .iter()
                .filter(|f| f.page == page && f.owner.is_none())
                .any(|f| {
                    f.logical_range[0] < span.logical_range[1]
                        && span.logical_range[0] < f.logical_range[1]
                })
            {
                return Err(fail("tagged OCR cannot also be paragraph replacement text"));
            }
            if !self.seen.insert(identity)
                || bound
                    .operands
                    .insert(
                        (occurrence.stream_index, occurrence.stream, occurrence.range),
                        *owner,
                    )
                    .is_some()
            {
                return Err(fail("tagged OCR source occurrence is ambiguous"));
            }
            bound
                .owners
                .entry(figure.clone())
                .or_default()
                .insert(*owner);
        }
        Ok(bound)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::tagged_structure::story) fn form(
        &mut self,
        page: usize,
        figure: &str,
        scope: ObjectRef,
        marks: &PageMarks,
        index: &StructureIndex,
        model: &MultiRunRangeModel,
    ) -> Result<PageOcr> {
        let mut bound = PageOcr::default();
        let spans = model
            .source_spans
            .iter()
            .map(|span| (span.span_id.as_str(), span))
            .collect::<BTreeMap<_, _>>();
        for occurrence in &marks.text {
            crate::cancel::check_current_cancel("tagged nested Figure OCR ownership")?;
            let Some(span) = model.source_spans.iter().find(|span| {
                (span.stream_object, span.stream_generation) == occurrence.stream
                    && span.byte_range == occurrence.range
            }) else {
                if !occurrence.bytes_empty {
                    return Err(fail(
                        "nested Figure text lacks exact Form-source provenance",
                    ));
                }
                continue;
            };
            let identity = (page, span.span_id.clone());
            let Some((expected_figure, expected_owner)) = self.expected.get(&identity) else {
                continue;
            };
            if expected_figure != figure {
                continue;
            }
            let owner = *expected_owner;
            if !spans.contains_key(span.span_id.as_str())
                || span.text_render_mode != 3
                || occurrence.bytes_empty
            {
                return Err(fail(
                    "tagged nested OCR selection is not its exact invisible Form operand",
                ));
            }
            let owners = occurrence
                .marks
                .iter()
                .filter_map(|slot| marks.marks[*slot].mcid)
                .map(|mcid| {
                    index
                        .marked
                        .get(&scope)
                        .and_then(|items| items.get(&mcid))
                        .copied()
                        .ok_or_else(|| fail("tagged nested OCR has unresolved MCID ownership"))
                })
                .collect::<Result<BTreeSet<_>>>()?;
            if owners != BTreeSet::from([owner]) {
                return Err(fail(
                    "nested OCR belongs to a different semantic owner; whole-Figure capture cannot flatten it",
                ));
            }
            for &slot in &occurrence.marks {
                let mark = &marks.marks[slot];
                if !mark.safe || mark.artifact || mark.frame.is_some() || mark.image_key.is_some() {
                    return Err(fail(
                        "tagged nested OCR has unapproved optional/artifact/frame ownership",
                    ));
                }
                if mark.actual_text {
                    bound
                        .actual
                        .entry(figure.to_owned())
                        .or_default()
                        .insert(slot);
                }
            }
            if !self.seen.insert(identity)
                || bound
                    .operands
                    .insert(
                        (occurrence.stream_index, occurrence.stream, occurrence.range),
                        owner,
                    )
                    .is_some()
            {
                return Err(fail("tagged nested OCR source occurrence is ambiguous"));
            }
            bound
                .owners
                .entry(figure.to_owned())
                .or_default()
                .insert(owner);
        }
        Ok(bound)
    }
    pub(in crate::tagged_structure::story) fn finish(&self) -> Result<()> {
        if self.seen.len() != self.expected.len() {
            return Err(fail(
                "tagged OCR selection contains a stale or unvisited source operand",
            ));
        }
        Ok(())
    }
}
