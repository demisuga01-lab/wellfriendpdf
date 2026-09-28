//! OpenType's static-instance GETVARIATION fallback and explicit review receipts.
//! Source code offsets stay intact. This is not arbitrary hint-program execution.
use super::{
    glyf_instance::OutlineStage,
    tt_bytecode::{self, Budget, Inventory, Owner, QueryKind, PROGRAM_LIMIT},
    variation_store::{bytes, u16_at, u32_at},
};
use crate::{Result, WellfriendError};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
type Tables = BTreeMap<[u8; 4], Arc<[u8]>>;
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("TrueType hint instance: {message}"))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewKind {
    InitializationVariationQuery,
    InitializationCallsWithVariationDefinitions,
    FontVariationCapabilityQuery,
    LegacyVariationOpcode,
    DynamicInstructionDefinition,
    InitializationRelativeControlFlow,
    UnknownInstructionSemantics,
}
#[derive(Debug)]
#[allow(dead_code)] // Review provenance is part of the retained audit model.
pub(crate) struct Review {
    pub owner: Owner,
    pub offset: Option<usize>,
    pub kind: ReviewKind,
}
#[allow(dead_code)] // Stage provenance is retained for audit/report consumers.
pub(crate) struct HintStage {
    /// prep and maxp replacements. maxp starts from the outline stage, not the
    /// original profile, so newly recomputed glyph maxima are not overwritten.
    pub tables: BTreeMap<[u8; 4], Vec<u8>>,
    pub programs: Vec<Inventory>,
    pub review: Vec<Review>,
    pub appended_definition: std::ops::Range<usize>,
    pub normalized_values: Vec<i16>,
    pub instruction_capacity: u16,
    pub stack_capacity: u16,
    pub scan_work: usize,
}
fn fallback(coordinates: &[ttf_parser::NormalizedCoordinate]) -> Vec<u8> {
    // The source instructions remain at their original byte offsets. 145 is
    // defined only after prep completes, without consuming source stack values.
    let mut out = vec![0xb0, 0x91, 0x89, 0x41, coordinates.len() as u8];
    for coordinate in coordinates {
        out.extend(coordinate.get().to_be_bytes());
    }
    out.push(0x2d);
    out
}
pub(crate) fn freeze(
    tables: &Tables,
    outlines: &OutlineStage,
    coordinates: &[ttf_parser::NormalizedCoordinate],
) -> Result<HintStage> {
    crate::cancel::check_current_cancel("TrueType hint stage")?;
    if coordinates.is_empty() || coordinates.len() > 64 {
        return Err(fail("requires one through 64 selected axes"));
    }
    let get = |tag: &[u8; 4]| {
        outlines
            .tables
            .get(tag)
            .ok_or_else(|| fail("missing staged outline owner"))
    };
    // Inspect the writer's exact ranges without cloning/reparsing all points.
    let glyf = get(b"glyf")?;
    let loca = get(b"loca")?;
    let glyph_count = u16_at(get(b"maxp")?, 4)?;
    let long = match u16_at(get(b"head")?, 50)? {
        0 => false,
        1 => true,
        _ => return Err(fail("invalid staged loca format")),
    };
    let location = |i: usize| -> Result<usize> {
        if long {
            Ok(u32_at(loca, i * 4)? as usize)
        } else {
            Ok(usize::from(u16_at(loca, i * 2)?) * 2)
        }
    };
    let mut budget = Budget::default();
    let mut programs = Vec::new();
    for (tag, owner) in [(b"fpgm", Owner::Font), (b"prep", Owner::Preparation)] {
        if let Some(data) = tables.get(tag) {
            programs.push(tt_bytecode::scan(data, owner, &mut budget)?);
        }
    }
    let mut last_glyph = None;
    for (id, range) in &outlines.instruction_ranges {
        crate::cancel::check_current_cancel("TrueType glyph program scan")?;
        if *id >= glyph_count || last_glyph.is_some_and(|previous| previous >= *id) {
            return Err(fail("unordered or invalid instruction glyph identities"));
        }
        let start = location(usize::from(*id))?;
        let end = location(usize::from(*id) + 1)?;
        if range.start < start.saturating_add(10) || range.end > end || range.start >= range.end {
            return Err(fail("instruction range outside its generated glyph"));
        }
        programs.push(tt_bytecode::scan(
            bytes(glyf, range.start, range.len())?,
            Owner::Glyph(*id),
            &mut budget,
        )?);
        last_glyph = Some(*id);
    }
    let mut review = Vec::new();
    let mut known_instructions = BTreeSet::new();
    let mut dynamic_idef = false;
    let mut maximum_literal_push = 0;
    let definitions_use_variation = programs.iter().any(|p| {
        p.queries
            .iter()
            .any(|q| q.in_definition && q.kind == QueryKind::Variation)
    });
    for p in &programs {
        crate::cancel::check_current_cancel("TrueType hint review")?;
        maximum_literal_push = maximum_literal_push.max(p.maximum_literal_push);
        for d in p.definitions.iter().filter(|d| d.instruction) {
            if let Some(opcode) = d.identifier {
                known_instructions.insert(opcode);
            } else {
                dynamic_idef = true;
                review.push(Review {
                    owner: p.owner,
                    offset: Some(d.offset),
                    kind: ReviewKind::DynamicInstructionDefinition,
                });
            }
        }
        for q in &p.queries {
            let kind = match q.kind {
                QueryKind::Variation if !matches!(p.owner, Owner::Glyph(_)) && !q.in_definition => {
                    Some(ReviewKind::InitializationVariationQuery)
                }
                QueryKind::Information
                    if q.information_selector
                        .is_none_or(|selector| selector & 8 != 0) =>
                {
                    Some(ReviewKind::FontVariationCapabilityQuery)
                }
                QueryKind::LegacyData => Some(ReviewKind::LegacyVariationOpcode),
                _ => None,
            };
            if let Some(kind) = kind {
                review.push(Review {
                    owner: p.owner,
                    offset: Some(q.offset),
                    kind,
                });
            }
        }
        if !matches!(p.owner, Owner::Glyph(_)) {
            if p.top_level_calls > 0 && definitions_use_variation {
                review.push(Review {
                    owner: p.owner,
                    offset: None,
                    kind: ReviewKind::InitializationCallsWithVariationDefinitions,
                });
            }
            if p.top_level_jumps > 0 {
                review.push(Review {
                    owner: p.owner,
                    offset: None,
                    kind: ReviewKind::InitializationRelativeControlFlow,
                });
            }
        }
        if p.unknown_opcodes > 0 {
            review.push(Review {
                owner: p.owner,
                offset: None,
                kind: ReviewKind::UnknownInstructionSemantics,
            });
        }
    }
    known_instructions.insert(0x91);
    let original_profile = get(b"maxp")?;
    bytes(original_profile, 0, 32)?;
    if u32_at(original_profile, 0)? != 0x10000 {
        return Err(fail("requires TrueType maxp version 1.0"));
    }
    let instruction_capacity = u16_at(original_profile, 22)?.max(if dynamic_idef {
        256
    } else {
        known_instructions.len() as u16
    });
    let original_stack = usize::from(u16_at(original_profile, 24)?).max(maximum_literal_push);
    let stack_capacity = u16::try_from(original_stack + coordinates.len().max(1))
        .map_err(|_| fail("conservative hint stack capacity exceeds maxp"))?;
    let mut maxp = original_profile.clone();
    maxp[22..24].copy_from_slice(&instruction_capacity.to_be_bytes());
    maxp[24..26].copy_from_slice(&stack_capacity.to_be_bytes());
    let old_prep = tables.get(b"prep").map_or(&[][..], |data| data.as_ref());
    let tail = fallback(coordinates);
    if old_prep.len() + tail.len() > PROGRAM_LIMIT {
        return Err(WellfriendError::ResourceLimit(
            "instanced prep exceeds 4 MiB".into(),
        ));
    }
    let mut prep = Vec::with_capacity(old_prep.len() + tail.len());
    for chunk in old_prep.chunks(65536) {
        crate::cancel::check_current_cancel("TrueType prep preservation")?;
        prep.extend_from_slice(chunk);
    }
    let appended_definition = prep.len()..prep.len() + tail.len();
    prep.extend(tail);
    let mut replacements = BTreeMap::new();
    replacements.insert(*b"prep", prep);
    replacements.insert(*b"maxp", maxp);
    crate::cancel::check_current_cancel("TrueType hint stage publication")?;
    Ok(HintStage {
        tables: replacements,
        programs,
        review,
        appended_definition,
        normalized_values: coordinates.iter().map(|n| n.get()).collect(),
        instruction_capacity,
        stack_capacity,
        scan_work: budget.work(),
    })
}

#[cfg(test)]
#[path = "tt_hint_instance_tests.rs"]
mod tests;
