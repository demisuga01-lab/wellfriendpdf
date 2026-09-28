//! Resolve CVT values without changing their indices or rounding each tuple.
//! This alone does not freeze TrueType instruction semantics or glyph outlines.
use super::{
    tuple_variations::{self, Budget, Domain, ITEM_LIMIT},
    variation_store::round_i32,
};
use crate::{Result, WellfriendError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CvtChange {
    pub index: u32,
    pub before: i16,
    pub after: i16,
}
#[allow(dead_code)] // Diagnostic fields are asserted by the variation corpus tests.
pub(crate) struct FrozenCvt {
    pub bytes: Vec<u8>,
    pub changes: Vec<CvtChange>,
    pub tuple_count: usize,
    pub active_tuple_count: usize,
    pub work: usize,
}
pub(crate) fn freeze(
    cvt: &[u8],
    cvar: &[u8],
    coordinates: &[ttf_parser::NormalizedCoordinate],
) -> Result<FrozenCvt> {
    crate::cancel::check_current_cancel("CVT instance preparation")?;
    if !cvt.len().is_multiple_of(2) {
        return Err(WellfriendError::invalid_input(
            "CVT has a partial FWORD value",
        ));
    }
    if cvt.len() / 2 > ITEM_LIMIT {
        return Err(WellfriendError::ResourceLimit(
            "CVT exceeds one million values".into(),
        ));
    }
    let mut budget = Budget::default();
    let deltas = tuple_variations::resolve(
        cvar,
        coordinates,
        &[],
        Domain::Cvt(cvt.len() / 2),
        &mut budget,
    )?;
    let mut out = Vec::with_capacity(cvt.len());
    let mut changes = Vec::new();
    budget.charge(deltas.values.len())?;
    for (index, (source, delta)) in cvt.chunks_exact(2).zip(&deltas.values).enumerate() {
        if index % 256 == 0 {
            crate::cancel::check_current_cancel("CVT value publication")?;
        }
        let before = i16::from_be_bytes(source.try_into().unwrap());
        let after = i16::try_from(round_i32(f64::from(before) + delta[0])?).map_err(|_| {
            WellfriendError::invalid_input("instanced CVT value exceeds FWORD range")
        })?;
        out.extend_from_slice(&after.to_be_bytes());
        if before != after {
            changes.push(CvtChange {
                index: index as u32,
                before,
                after,
            });
        }
    }
    crate::cancel::check_current_cancel("CVT stage publication")?;
    Ok(FrozenCvt {
        bytes: out,
        changes,
        tuple_count: deltas.tuples,
        active_tuple_count: deltas.active_tuples,
        work: budget.work,
    })
}

#[cfg(test)]
#[path = "cvar_instance_tests.rs"]
mod tests;
