//! Scalar soft-mask transfer preparation and a cumulatively bounded LUT.
use super::{FunctionResources, PreparedFunction, MAX_FUNCTION_WORK};
use crate::object::PdfObject;
use crate::reader::PdfReader;
use std::sync::Arc;

pub(crate) struct PreparedTransfer {
    function: Option<Arc<PreparedFunction>>,
    _memory: Option<crate::decode_scheduler::DecodeMemoryToken>,
}

impl PreparedTransfer {
    #[cfg(test)]
    pub(crate) fn prepare(object: &PdfObject, reader: &PdfReader) -> Option<Self> {
        Self::prepare_with_resources(object, reader, FunctionResources::default())
    }

    pub(crate) fn prepare_with_resources(
        object: &PdfObject,
        reader: &PdfReader,
        resources: FunctionResources<'_>,
    ) -> Option<Self> {
        crate::cancel::check_current_cancel("PDF transfer preparation").ok()?;
        if matches!(object, PdfObject::Name(name) if name == "Identity") {
            return Some(Self {
                function: None,
                _memory: None,
            });
        }
        if let Some(lease) =
            PreparedFunction::cached_with_resources(object, 1, false, reader, resources)
        {
            let (function, memory) = lease.into_parts();
            return (function.output_count() == 1).then_some(Self {
                function: Some(function),
                _memory: memory,
            });
        }
        // Identity may itself be indirect. Resolve only after the graph-cache
        // path, so repeated valid function references don't reparse the source.
        if matches!(object, PdfObject::Reference { .. })
            && matches!(reader.resolve(object.clone()).ok()?, PdfObject::Name(name) if name == "Identity")
        {
            crate::cancel::check_current_cancel("PDF transfer identity").ok()?;
            return Some(Self {
                function: None,
                _memory: None,
            });
        }
        None
    }

    pub(crate) fn is_identity(&self) -> bool {
        self.function.is_none()
    }

    pub(crate) fn evaluate(&self, input: f64) -> Option<f64> {
        let mut remaining = MAX_FUNCTION_WORK;
        self.evaluate_with_budget(input, &mut remaining)
    }

    fn evaluate_with_budget(&self, input: f64, remaining: &mut usize) -> Option<f64> {
        crate::cancel::check_current_cancel("PDF transfer evaluation").ok()?;
        if !input.is_finite() {
            return None;
        }
        let value = if let Some(function) = &self.function {
            let (output, used) = function.evaluate_metered(&[input], *remaining);
            *remaining = remaining.checked_sub(used)?;
            if output.len() != 1 {
                return None;
            }
            output[0]
        } else {
            *remaining = remaining.checked_sub(1)?;
            input
        };
        value.is_finite().then(|| value.clamp(0.0, 1.0))
    }

    pub(crate) fn lookup_table(&self) -> Option<[u8; 256]> {
        self.lookup_table_with_budget(MAX_FUNCTION_WORK)
    }

    fn lookup_table_with_budget(&self, mut remaining: usize) -> Option<[u8; 256]> {
        let mut table = [0u8; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let value = self.evaluate_with_budget(index as f64 / 255.0, &mut remaining)?;
            *slot = (value * 255.0).round() as u8;
        }
        Some(table)
    }
}

#[cfg(test)]
#[path = "transfer_function_tests.rs"]
mod tests;
