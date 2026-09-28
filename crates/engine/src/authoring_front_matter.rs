//! Transactional front-matter staging for fresh authoring. Front pages are
//! built in an isolated flow, parity padded, then spliced before the existing
//! body while every stored page/section authority is shifted exactly once.
use super::*;

#[cfg(test)]
#[path = "authoring_front_matter_tests.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq)]
pub struct FrontMatterReport {
    pub content_pages: usize,
    pub inserted_pages: usize,
    pub inserted_sections: usize,
    /// One-based physical page of the suppressed parity blank, when required.
    pub parity_blank_page: Option<usize>,
    /// One-based physical page on which the pre-existing body now begins.
    pub body_start_page: usize,
    pub anchor_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrontMatterTableOfContentsReport {
    pub table_of_contents: TableOfContentsReport,
    pub content_pages: usize,
    pub inserted_pages: usize,
    /// One-based physical page of the suppressed parity blank, when required.
    pub parity_blank_page: Option<usize>,
    /// One-based physical page on which the pre-existing body now begins.
    pub body_start_page: usize,
}

pub(super) fn prepend<F>(
    flow: &mut FlowDocument,
    section: FlowSection,
    author: F,
) -> Result<FrontMatterReport>
where
    F: FnOnce(&mut FlowDocument) -> Result<()>,
{
    if flow.builder.fields_materialized
        || flow.builder.notes_materialized
        || flow.builder.section_masters_materialized
    {
        return Err(WellfriendError::invalid_input(
            "front matter must be inserted before final authoring materialization",
        ));
    }
    // Validate the current body and stage every fallible layout operation away
    // from it. This also proves that each existing page has a section owner.
    sections::assignments(&flow.builder)?;
    let mut staged = FlowDocument::from_section(section.clone())?;
    staged.builder.custom_fonts = Arc::clone(&flow.builder.custom_fonts);
    staged.builder.font_stacks = Arc::clone(&flow.builder.font_stacks);
    staged.builder.images = flow.builder.images.clone();
    staged.builder.metadata = flow.builder.metadata.clone();
    staged.builder.language = flow.builder.language.clone();
    for page in &mut staged.builder.pages {
        page.custom_fonts = Arc::clone(&flow.builder.custom_fonts);
        page.font_stacks = Arc::clone(&flow.builder.font_stacks);
    }
    staged.builder.outline = flow.builder.outline.clone();
    staged.builder.outline_item_count = flow.builder.outline_item_count;
    staged.builder.next_field_plan_id = flow.builder.next_field_plan_id;
    staged.builder.next_footnote_id = flow.builder.next_footnote_id;
    staged.builder.next_structure_id = flow.builder.next_structure_id;

    author(&mut staged)?;
    if staged.builder.metadata != flow.builder.metadata {
        return Err(WellfriendError::UnsupportedFeature(
            "front-matter staging cannot mutate document metadata".into(),
        ));
    }
    if staged.builder.language != flow.builder.language {
        return Err(WellfriendError::UnsupportedFeature(
            "front-matter staging cannot mutate the document language".into(),
        ));
    }
    if staged.builder.outline != flow.builder.outline
        || staged.builder.outline_item_count != flow.builder.outline_item_count
    {
        return Err(WellfriendError::UnsupportedFeature(
            "front-matter staging cannot mutate the document outline".into(),
        ));
    }
    sections::assignments(&staged.builder)?;
    for page in &staged.builder.pages {
        if !page.footnotes.is_empty()
            && page
                .section_index
                .and_then(|index| staged.builder.sections.get(index))
                .is_some_and(|section| {
                    section.footnote_numbering.scope == NoteNumberScope::Document
                })
        {
            return Err(WellfriendError::UnsupportedFeature(
                "front-matter footnotes must use section-scoped numbering so existing body notes remain stable"
                    .into(),
            ));
        }
    }
    fields::validate_anchor_shift(&staged.builder, 0, 0)?;
    let content_pages = staged.builder.pages.len();
    let parity_blank_page = if content_pages % 2 == 1 {
        staged.add_page_break();
        staged.current_page_mut().suppress_section_master = true;
        Some(staged.builder.pages.len())
    } else {
        None
    };
    sections::assignments(&staged.builder)?;
    let inserted_pages = staged.builder.pages.len();
    if inserted_pages == 0 || inserted_pages % 2 != 0 {
        return Err(WellfriendError::invalid_input(
            "front-matter staging did not preserve body-page parity",
        ));
    }
    let final_page_count = flow
        .builder
        .pages
        .len()
        .checked_add(inserted_pages)
        .ok_or_else(|| WellfriendError::ResourceLimit("front-matter page count overflow".into()))?;
    if flow.current_page >= flow.builder.pages.len()
        || flow.builder.pages[flow.current_page].section_index != Some(flow.current_section)
        || flow.page_size != flow.builder.pages[flow.current_page].size
        || flow.margins != flow.builder.pages[flow.current_page].margins
    {
        return Err(WellfriendError::invalid_input(
            "active authoring flow page ownership is invalid before front-matter insertion",
        ));
    }
    let inserted_sections = staged.builder.sections.len();
    if inserted_sections == 0 {
        return Err(WellfriendError::invalid_input(
            "front-matter staging produced no section",
        ));
    }

    let shifted_current_page = flow
        .current_page
        .checked_add(inserted_pages)
        .ok_or_else(|| WellfriendError::ResourceLimit("front-matter page shift overflow".into()))?;
    let shifted_current_section = flow
        .current_section
        .checked_add(inserted_sections)
        .ok_or_else(|| {
            WellfriendError::ResourceLimit("front-matter section shift overflow".into())
        })?;
    let body_start_page = inserted_pages
        .checked_add(1)
        .ok_or_else(|| WellfriendError::ResourceLimit("front-matter body page overflow".into()))?;
    flow.builder
        .sections
        .len()
        .checked_add(inserted_sections)
        .ok_or_else(|| {
            WellfriendError::ResourceLimit("front-matter section count overflow".into())
        })?;
    for page in &flow.builder.pages {
        page.section_index
            .and_then(|index| index.checked_add(inserted_sections))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit("front-matter page section shift overflow".into())
            })?;
    }
    for name in staged.builder.anchors.keys() {
        if flow.builder.anchors.contains_key(name) {
            return Err(WellfriendError::invalid_input(
                "front-matter anchor name conflicts with an existing body anchor",
            ));
        }
    }
    fields::validate_anchor_shift(&flow.builder, inserted_pages, inserted_sections)?;
    notes::validate_reference_page_shift(&flow.builder.pages, inserted_pages)?;

    // All checks above are complete. The remaining splice contains no fallible
    // document operation, so the body cannot be left partially shifted.
    let staged_next_field_plan_id = staged.builder.next_field_plan_id;
    let staged_next_footnote_id = staged.builder.next_footnote_id;
    let staged_next_structure_id = staged.builder.next_structure_id;
    let staged_custom_fonts = Arc::clone(&staged.builder.custom_fonts);
    let staged_font_stacks = Arc::clone(&staged.builder.font_stacks);
    let staged_images = std::mem::take(&mut staged.builder.images);
    let front_sections = std::mem::take(&mut staged.builder.sections);
    let front_anchors = std::mem::take(&mut staged.builder.anchors);
    let front_structures = std::mem::take(&mut staged.builder.structures);
    let anchor_names = front_anchors.keys().cloned().collect::<Vec<_>>();
    let mut front_pages = std::mem::take(&mut staged.builder.pages);
    let mut body_pages = std::mem::take(&mut flow.builder.pages);
    for page in &mut body_pages {
        page.section_index = page.section_index.map(|index| index + inserted_sections);
    }
    notes::apply_reference_page_shift(&mut body_pages, inserted_pages);
    front_pages.append(&mut body_pages);
    debug_assert_eq!(front_pages.len(), final_page_count);
    flow.builder.pages = front_pages;
    drop(flow.builder.sections.splice(0..0, front_sections));
    fields::apply_anchor_shift(&mut flow.builder, inserted_pages, inserted_sections);
    flow.builder.anchors.extend(front_anchors);
    let mut body_structures = std::mem::take(&mut flow.builder.structures);
    let mut structures = front_structures;
    structures.append(&mut body_structures);
    flow.builder.structures = structures;
    flow.builder.custom_fonts = staged_custom_fonts;
    flow.builder.font_stacks = staged_font_stacks;
    flow.builder.images = staged_images;
    for page in &mut flow.builder.pages {
        page.custom_fonts = Arc::clone(&flow.builder.custom_fonts);
        page.font_stacks = Arc::clone(&flow.builder.font_stacks);
    }
    flow.builder.next_field_plan_id = staged_next_field_plan_id;
    flow.builder.next_footnote_id = staged_next_footnote_id;
    flow.builder.next_structure_id = staged_next_structure_id;
    flow.current_page = shifted_current_page;
    flow.current_section = shifted_current_section;

    Ok(FrontMatterReport {
        content_pages,
        inserted_pages,
        inserted_sections,
        parity_blank_page,
        body_start_page,
        anchor_names,
    })
}

pub(super) fn prepend_table_of_contents(
    flow: &mut FlowDocument,
    section: FlowSection,
    style: &TableOfContentsStyle,
) -> Result<FrontMatterTableOfContentsReport> {
    let mut table_of_contents = None;
    let report = prepend(flow, section, |front| {
        table_of_contents = Some(front.add_table_of_contents(style)?);
        Ok(())
    })?;
    let table_of_contents =
        table_of_contents.expect("successful front-matter TOC closure always records its report");
    Ok(FrontMatterTableOfContentsReport {
        table_of_contents,
        content_pages: report.content_pages,
        inserted_pages: report.inserted_pages,
        parity_blank_page: report.parity_blank_page,
        body_start_page: report.body_start_page,
    })
}
