//! Reuse one constraint/pagination engine in logical flow coordinates, then
//! return physical PDF geometry before tagging, anchoring, saving or previewing.
use super::*;
use crate::advanced_editing::StoryDecoration;
#[cfg(test)]
#[path = "story_writing_mode_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
struct Axes {
    rect: [f64; 4],
    mode: WritingMode,
}
impl Axes {
    fn flow_rect(self) -> [f64; 4] {
        [
            0.0,
            0.0,
            self.rect[3] - self.rect[1],
            self.rect[2] - self.rect[0],
        ]
    }
    fn physical(self, p: [f64; 2]) -> [f64; 2] {
        [
            if self.mode == WritingMode::VerticalRl {
                self.rect[0] + p[1]
            } else {
                self.rect[2] - p[1]
            },
            self.rect[3] - p[0],
        ]
    }
    fn flow(self, p: [f64; 2]) -> [f64; 2] {
        [
            self.rect[3] - p[1],
            if self.mode == WritingMode::VerticalRl {
                p[0] - self.rect[0]
            } else {
                self.rect[2] - p[0]
            },
        ]
    }
    fn rectangle(self, r: [f64; 4], to_physical: bool) -> [f64; 4] {
        let point = |p| {
            if to_physical {
                self.physical(p)
            } else {
                self.flow(p)
            }
        };
        let a = point([r[0], r[1]]);
        let b = point([r[2], r[3]]);
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        ]
    }
    fn preview_frame(self, frame: &mut StoryFrameLayout, to_physical: bool) {
        let point = |p| {
            if to_physical {
                self.physical(p)
            } else {
                self.flow(p)
            }
        };
        for line in &mut frame.lines {
            let p = point([line.x, line.baseline]);
            line.x = p[0];
            line.baseline = p[1];
        }
        for decoration in &mut frame.decorations {
            match decoration {
                StoryDecoration::Fill { rect, .. } => *rect = self.rectangle(*rect, to_physical),
                StoryDecoration::Stroke { from, to, .. } => {
                    *from = point(*from);
                    *to = point(*to);
                }
            }
        }
        for cell in &mut frame.table_cells {
            cell.rect = self.rectangle(cell.rect, to_physical);
        }
        for figure in &mut frame.figures {
            figure.rect = self.rectangle(figure.rect, to_physical);
        }
        frame.frame.rect = if to_physical {
            self.rect
        } else {
            self.flow_rect()
        };
        for rect in &mut frame.frame.exclusions {
            *rect = self.rectangle(*rect, to_physical);
        }
    }
}

pub(super) fn layout(
    request: &LinkedStoryRequest,
    fonts: &[ApprovedFontAsset],
    indices: &[usize],
    choices: Vec<StoryFontChoice>,
    seed: Option<LayoutSeed<'_>>,
) -> Result<LinkedStoryPreview> {
    if !request.writing_mode.is_vertical() {
        return layout_seeded_flow(request, fonts, indices, choices, seed);
    }
    // Avoid cloning the approved font byte pool merely to change geometry.
    let mut flow = LinkedStoryRequest {
        writing_mode: request.writing_mode,
        story_id: request.story_id.clone(),
        input_sha256: request.input_sha256.clone(),
        frames: request.frames.clone(),
        paragraphs: request.paragraphs.clone(),
        fonts: Vec::new(),
        annotation_anchors: Vec::new(),
        figures: request.figures.clone(),
        figure_removals: Vec::new(),
        figure_detachments: Vec::new(),
        source_tags: request.source_tags.clone(),
        table_layout: request.table_layout.clone(),
        allow_font_substitution: request.allow_font_substitution,
        allow_page_creation: request.allow_page_creation,
        prune_empty_pages: request.prune_empty_pages,
        max_new_pages: request.max_new_pages,
        mode: request.mode,
        signature_policy_override: request.signature_policy_override,
    };
    let geometry = request
        .frames
        .iter()
        .map(|f| {
            (
                f.id.as_str(),
                Axes {
                    rect: f.rect,
                    mode: request.writing_mode,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let template = Axes {
        rect: request
            .frames
            .last()
            .ok_or_else(|| fail("vertical story has no frames"))?
            .rect,
        mode: request.writing_mode,
    };
    for frame in &mut flow.frames {
        let axes = geometry[frame.id.as_str()];
        frame.rect = axes.flow_rect();
        for rect in &mut frame.exclusions {
            *rect = axes.rectangle(*rect, false);
        }
    }
    // Images remain upright physical rectangles. Their footprint swaps axes;
    // gap/alignment are block-axis/inline-axis layout constraints respectively.
    for figure in &mut flow.figures {
        std::mem::swap(&mut figure.width, &mut figure.height);
    }
    let mut previous: Option<LinkedStoryPreview> = seed.as_ref().map(|s| s.previous.clone());
    if let Some(previous) = &mut previous {
        for frame in &mut previous.frames {
            let axes = Axes {
                rect: frame.frame.rect,
                mode: request.writing_mode,
            };
            axes.preview_frame(frame, false);
        }
    }
    let mapped_seed = match (&seed, &previous) {
        (Some(seed), Some(previous)) => Some(LayoutSeed {
            previous,
            paragraph_hashes: seed.paragraph_hashes,
            font_indices: seed.font_indices,
        }),
        _ => None,
    };
    let mut preview = layout_seeded_flow(&flow, fonts, indices, choices, mapped_seed)?;
    for frame in &mut preview.frames {
        crate::cancel::check_current_cancel("story flow-to-page geometry")?;
        let axes = geometry
            .get(frame.frame.id.as_str())
            .copied()
            .unwrap_or(template);
        axes.preview_frame(frame, true);
    }
    preview.exact_limits.push("Vertical flow is top-to-bottom inline progression with explicit right-to-left or left-to-right column progression; ruby, tate-chu-yoko and custom kinsoku tailoring remain separate".into());
    Ok(preview)
}
