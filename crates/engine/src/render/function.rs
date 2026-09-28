//! PDF function evaluation (spec §7.10) for shadings, color spaces, and
//! soft-mask transfer functions.
//!
//! Supported function types:
//! - Type 0 (sampled): bounded tensor-product linear/cubic interpolation.
//!   Cubic axes with fewer than four samples use linear interpolation.
//!   Current source regression additions are not runtime qualification.
//! - Type 2 (exponential interpolation) and Type 3 (stitching): immutable,
//!   domain/range-aware graphs shared with the retained shading evaluator.
//! - Type 4 (PostScript calculator): a small stack-based interpreter for the
//!   restricted PostScript subset defined in spec Tables 42–44.
//!
//! The public entry point is `eval_function_n`, which takes a slice of inputs
//! (so 2-input functions used by ShadingType 1 and mesh shadings work), returning
//! the output components. A single-input convenience wrapper lives in
//! `crate::render::shading::eval_function`.
#![allow(dead_code)] // Legacy test adapters; runtime rendering uses prepared graphs.

use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

pub(crate) const MAX_TYPE0_SAMPLE_VALUES: usize = 4_194_304;
pub(crate) const MAX_TYPE0_INTERPOLATION_DIMENSIONS: usize = 8;
pub(crate) const MAX_TYPE4_TOKENS: usize = 16_384;
pub(crate) const MAX_TYPE4_STACK: usize = 1_024;
const MAX_FUNCTION_ARRAY_COMPONENTS: usize = crate::render::colorspace::MAX_DEVICEN_COMPONENTS;
const MAX_FUNCTION_VISITS: usize = 4096;

#[cfg(test)]
#[path = "function_domain_tests.rs"]
mod domain_tests;

#[path = "sampled_function.rs"]
mod sampled;

#[path = "prepared_function.rs"]
mod prepared;
pub(crate) use prepared::PreparedFunction;

#[path = "function_cache.rs"]
mod cache;
pub(crate) use cache::FunctionCache;
pub use cache::FunctionCacheMetrics;

#[path = "function_resources.rs"]
mod resources;
pub(crate) use resources::{FunctionLease, FunctionResources};

#[path = "transfer_function.rs"]
mod transfer;
pub(crate) use transfer::PreparedTransfer;

// Shared across stitching children and component arrays, not reset by recursion.
const MAX_FUNCTION_WORK: usize = 8_388_608;
const MAX_TYPE4_PROGRAM_BYTES: usize = 1_048_576;

fn charge_work(remaining: &mut usize, amount: usize) -> Option<()> {
    *remaining = remaining.checked_sub(amount)?;
    crate::cancel::check_current_cancel("PDF function work budget").ok()
}

/// Evaluate a PDF function with one or more inputs, returning its output
/// components. Returns an empty `Vec` for unsupported types or malformed input
/// (never panics).
pub(crate) fn eval_function_n(
    func_obj: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
) -> Vec<f64> {
    if !inputs.iter().all(|value| value.is_finite()) {
        return Vec::new();
    }
    PreparedFunction::cached_single(func_obj, inputs.len(), reader)
        .map_or_else(Vec::new, |function| function.evaluate(inputs))
}

/// Evaluate a shading `/Function`, accepting either one normal PDF function or
/// a PDF array of one-output component functions.
pub(crate) fn eval_function_or_array_n(
    func_obj: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
) -> Vec<f64> {
    if !inputs.iter().all(|value| value.is_finite()) {
        return Vec::new();
    }
    PreparedFunction::cached(func_obj, inputs.len(), reader)
        .map_or_else(Vec::new, |function| function.evaluate(inputs))
}

/// Validate the dictionary fields that `eval_function_n` would otherwise
/// tolerate through optional defaults. This is used by active render paths that
/// must fail closed on malformed present fields instead of sampling defaults.
pub(crate) fn validate_function_shape(
    func_obj: &PdfObject,
    input_count: usize,
    reader: &PdfReader,
) -> bool {
    let mut remaining = MAX_FUNCTION_VISITS;
    input_count > 0
        && validate_function_shape_inner(func_obj, input_count, reader, 0, &mut remaining).is_some()
}

/// Validate a shading `/Function`, accepting either one function or an array of
/// component functions. Array entries are still evaluated later to prove each
/// component function has exactly one finite output.
pub(crate) fn validate_function_or_array_shape(
    func_obj: &PdfObject,
    input_count: usize,
    reader: &PdfReader,
) -> bool {
    if input_count == 0 {
        return false;
    }
    let mut remaining = MAX_FUNCTION_VISITS;
    let Some(functions) = resolve_to_array(func_obj, reader) else {
        return validate_function_shape_inner(func_obj, input_count, reader, 0, &mut remaining)
            .is_some();
    };
    !functions.is_empty()
        && functions.len() <= MAX_FUNCTION_ARRAY_COMPONENTS
        && functions.iter().all(|function| {
            validate_function_shape_inner(function, input_count, reader, 0, &mut remaining)
                == Some(1)
        })
}

fn validate_function_shape_inner(
    func_obj: &PdfObject,
    input_count: usize,
    reader: &PdfReader,
    depth: usize,
    remaining: &mut usize,
) -> Option<usize> {
    if depth > 16
        || *remaining == 0
        || crate::cancel::check_current_cancel("PDF function shape traversal").is_err()
    {
        return None;
    }
    *remaining -= 1;
    let dict = resolve_to_dict(func_obj, reader)?;
    let domain = ordered_pairs(&dict, "Domain", true)??;
    if domain.len() != input_count.checked_mul(2)? {
        return None;
    }
    match dict.get_integer("FunctionType")? {
        0 => validate_type0_shape(&dict),
        2 => validate_type2_shape(&dict),
        3 => validate_type3_shape(&dict, reader, depth + 1, remaining),
        4 => validate_type4_shape(&dict, input_count),
        _ => None,
    }
}

fn validate_type0_shape(dict: &PdfDictionary) -> Option<usize> {
    let size = strict_type0_size(dict)?;
    strict_type0_order(dict)?;
    let range = ordered_pairs(dict, "Range", true)??;
    if size
        .iter()
        .try_fold(range.len() / 2, |count, &axis| count.checked_mul(axis))?
        > MAX_TYPE0_SAMPLE_VALUES
    {
        return None;
    }
    ordered_pairs(dict, "Domain", true)??;
    require_strict_float_array_exact(dict, "Domain", size.len().checked_mul(2)?)?;
    let bps = usize::try_from(dict.get_integer("BitsPerSample")?).ok()?;
    if !matches!(bps, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) {
        return None;
    }
    if let Some(encode) = strict_float_array_field(dict, "Encode").ok()? {
        if encode.len() != size.len().checked_mul(2)? {
            return None;
        }
    }
    if let Some(decode) = strict_float_array_field(dict, "Decode").ok()? {
        if decode.len() != range.len() {
            return None;
        }
    }
    Some(range.len() / 2)
}

fn strict_type0_order(dict: &PdfDictionary) -> Option<sampled::Order> {
    match dict.get("Order") {
        None | Some(PdfObject::Null) | Some(PdfObject::Integer(1)) => Some(sampled::Order::Linear),
        Some(PdfObject::Integer(3)) => Some(sampled::Order::Cubic),
        _ => None,
    }
}

fn strict_type0_size(dict: &PdfDictionary) -> Option<Vec<usize>> {
    let size_obj = dict.get("Size")?.as_array()?;
    if size_obj.is_empty() || size_obj.len() > MAX_TYPE0_INTERPOLATION_DIMENSIONS {
        return None;
    }
    let mut size = Vec::with_capacity(size_obj.len());
    for item in size_obj {
        let value = item.as_integer()?;
        if value <= 0 {
            return None;
        }
        size.push(usize::try_from(value).ok()?);
    }
    Some(size)
}

fn validate_type2_shape(dict: &PdfDictionary) -> Option<usize> {
    let domain = ordered_pairs(dict, "Domain", true)??;
    if domain.len() != 2 {
        return None;
    }
    let n = dict.get("N")?.as_number()?;
    if !n.is_finite()
        || (n.fract() != 0.0 && domain[0] < 0.0)
        || (n < 0.0 && domain[0] <= 0.0 && domain[1] >= 0.0)
    {
        return None;
    }
    let mut count = None;
    for key in ["C0", "C1"] {
        let n = strict_float_array_field(dict, key)
            .ok()?
            .map_or(1, |v| v.len());
        if n == 0 || count.is_some_and(|previous| previous != n) {
            return None;
        }
        count = Some(n);
    }
    let count = count?;
    if ordered_pairs(dict, "Range", false)?.is_some_and(|r| r.len() != 2 * count) {
        return None;
    }
    Some(count)
}

fn validate_type3_shape(
    dict: &PdfDictionary,
    reader: &PdfReader,
    next_depth: usize,
    remaining: &mut usize,
) -> Option<usize> {
    let domain = ordered_pairs(dict, "Domain", true)??;
    if domain.len() != 2 {
        return None;
    }
    let functions = dict.get("Functions")?.as_array()?;
    if functions.is_empty()
        || functions.len() > super::parameter_dictionary::MAX_STITCHING_FUNCTIONS
        || (functions.len() > 1 && domain[0] >= domain[1])
    {
        return None;
    }
    let bounds =
        require_strict_float_array_exact(dict, "Bounds", functions.len().saturating_sub(1))?;
    let mut previous = domain[0];
    for bound in bounds {
        if bound <= previous || bound > domain[1] {
            return None;
        }
        previous = bound;
    }
    require_strict_float_array_exact(dict, "Encode", functions.len().checked_mul(2)?)?;
    let mut count = None;
    for function in functions {
        let n = validate_function_shape_inner(function, 1, reader, next_depth, remaining)?;
        if count.is_some_and(|previous| previous != n) {
            return None;
        }
        count = Some(n);
    }
    let count = count?;
    if ordered_pairs(dict, "Range", false)?.is_some_and(|r| r.len() != 2 * count) {
        return None;
    }
    Some(count)
}

fn validate_type4_shape(dict: &PdfDictionary, input_count: usize) -> Option<usize> {
    require_strict_float_array_exact(dict, "Domain", input_count.checked_mul(2)?)?;
    ordered_pairs(dict, "Domain", true)??;
    let range = ordered_pairs(dict, "Range", true)??;
    Some(range.len() / 2)
}

fn ordered_pairs(dict: &PdfDictionary, key: &str, required: bool) -> Option<Option<Vec<f64>>> {
    let values = strict_float_array_field(dict, key).ok()?;
    match values {
        None if required => None,
        None => Some(None),
        Some(values)
            if !values.is_empty()
                && values.len().is_multiple_of(2)
                && values.chunks_exact(2).all(|p| p[0] <= p[1]) =>
        {
            Some(Some(values))
        }
        _ => None,
    }
}

pub(super) fn clip_output(dict: &PdfDictionary, mut output: Vec<f64>) -> Option<Vec<f64>> {
    if output.is_empty() || !output.iter().all(|v| v.is_finite()) {
        return None;
    }
    if let Some(range) = ordered_pairs(dict, "Range", false)? {
        if range.len() != output.len().checked_mul(2)? {
            return None;
        }
        for (i, value) in output.iter_mut().enumerate() {
            *value = value.clamp(range[2 * i], range[2 * i + 1]);
        }
    }
    Some(output)
}

/// Relative position in an ordered, already clipped domain. No absolute cutoff
/// is appropriate: tiny domains may map onto the entire sample lattice.
pub(super) fn domain_position(x: f64, lo: f64, hi: f64) -> Option<f64> {
    if ![x, lo, hi].iter().all(|v| v.is_finite()) || lo > hi {
        return None;
    }
    if lo == hi {
        return Some(0.0);
    }
    let x = x.clamp(lo, hi);
    let width = hi - lo;
    let position = if width.is_finite() {
        (x - lo) / width
    } else {
        (x * 0.5 - lo * 0.5) / (hi * 0.5 - lo * 0.5)
    };
    position.is_finite().then(|| position.clamp(0.0, 1.0))
}

fn strict_float_array_field(
    dict: &PdfDictionary,
    key: &str,
) -> std::result::Result<Option<Vec<f64>>, ()> {
    let Some(value) = dict.get(key) else {
        return Ok(None);
    };
    let arr = value.as_array().ok_or(())?;
    let mut values = Vec::with_capacity(arr.len());
    for item in arr {
        let value = item.as_number().ok_or(())?;
        if !value.is_finite() {
            return Err(());
        }
        values.push(value);
    }
    Ok(Some(values))
}

fn require_strict_float_array_exact(
    dict: &PdfDictionary,
    key: &str,
    len: usize,
) -> Option<Vec<f64>> {
    let values = strict_float_array_field(dict, key).ok()??;
    if values.len() == len {
        Some(values)
    } else {
        None
    }
}

/// Resolve a function reference / dict / stream to its dictionary.
fn resolve_to_dict(obj: &PdfObject, reader: &PdfReader) -> Option<PdfDictionary> {
    let resolved = match obj {
        PdfObject::Reference { .. } => std::borrow::Cow::Owned(reader.resolve(obj.clone()).ok()?),
        _ => std::borrow::Cow::Borrowed(obj),
    };
    let dict = match resolved.as_ref() {
        PdfObject::Dictionary(d) | PdfObject::Stream { dict: d, .. } => d,
        _ => return None,
    };
    Some(
        super::parameter_dictionary::function(dict, Some(reader))
            .ok()?
            .into_owned(),
    )
}

fn resolve_to_array(obj: &PdfObject, reader: &PdfReader) -> Option<Vec<PdfObject>> {
    match obj {
        PdfObject::Array(items) => Some(items.clone()),
        PdfObject::Reference { .. } => match reader.resolve(obj.clone()).ok()? {
            PdfObject::Array(items) => Some(items),
            _ => None,
        },
        _ => None,
    }
}

/// Resolve a function object to its decoded stream bytes (Type 0 samples or
/// Type 4 program text), applying any stream filters.
fn resolve_stream_bytes_limited(
    obj: &PdfObject,
    reader: &PdfReader,
    max_bytes: Option<usize>,
) -> Option<Vec<u8>> {
    let stream = match obj {
        PdfObject::Stream { .. } => std::borrow::Cow::Borrowed(obj),
        PdfObject::Reference { .. } => match reader.resolve(obj.clone()).ok()? {
            s @ PdfObject::Stream { .. } => std::borrow::Cow::Owned(s),
            _ => return None,
        },
        _ => return None,
    };
    let mut limits = crate::filters::DecodeLimits::default();
    if let Some(max_bytes) = max_bytes {
        limits.max_decoded_bytes_per_stream =
            limits.max_decoded_bytes_per_stream.min(max_bytes as u64);
    }
    let decoded = crate::filters::decode_stream_with_limits(&stream, reader, &limits).ok()?;
    if max_bytes.is_some_and(|limit| decoded.len() > limit) {
        return None;
    }
    Some(decoded)
}

// ---------------------------------------------------------------------------
// MSB-first bit reader supporting up to 32-bit fields
// ---------------------------------------------------------------------------

/// Reads big-endian bit fields of width 1..=32 from a byte slice. Shared by
/// Function Type 0 sample unpacking and mesh-shading vertex unpacking.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Absolute bit position from the start of `data`.
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    /// Read `bits` (1..=32) as an unsigned value. Returns `None` past EOF.
    pub(crate) fn read(&mut self, bits: usize) -> Option<u32> {
        if bits == 0 || bits > 32 {
            return None;
        }
        let mut value: u64 = 0;
        for _ in 0..bits {
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = 7 - (self.bit_pos % 8);
            let byte = *self.data.get(byte_idx)?;
            let bit = (byte >> bit_in_byte) & 1;
            value = (value << 1) | bit as u64;
            self.bit_pos += 1;
        }
        Some(value as u32)
    }

    /// Discard bits up to the next byte boundary (mesh shadings byte-align each
    /// vertex/flag group per the spec). Used by the mesh-shading vertex reader.
    pub(crate) fn align_to_byte(&mut self) {
        if !self.bit_pos.is_multiple_of(8) {
            self.bit_pos = (self.bit_pos / 8 + 1) * 8;
        }
    }

    /// Number of unread bits remaining. Used by the mesh-shading vertex reader
    /// to detect the end of the vertex/patch stream.
    pub(crate) fn bits_remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.bit_pos)
    }
}

/// Maximum unsigned value representable in `bits` bits, as f64.
pub(crate) fn max_value(bits: usize) -> f64 {
    if bits >= 32 {
        u32::MAX as f64
    } else {
        ((1u64 << bits) - 1) as f64
    }
}

// ---------------------------------------------------------------------------
// Type 0: sampled functions
// ---------------------------------------------------------------------------

/// Read the `index`-th sample (each `bps` bits) from the packed sample stream.
fn read_sample(data: &[u8], index: usize, bps: usize) -> Option<f64> {
    if !matches!(bps, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32) {
        return None;
    }
    let bit_start = index.checked_mul(bps)?;
    let byte_start = bit_start / 8;
    let leading_bits = bit_start % 8;
    let byte_count = (leading_bits + bps).div_ceil(8);
    let bytes = data.get(byte_start..byte_start.checked_add(byte_count)?)?;
    let word = bytes
        .iter()
        .fold(0_u64, |value, &byte| (value << 8) | u64::from(byte));
    let trailing_bits = byte_count * 8 - leading_bits - bps;
    Some(((word >> trailing_bits) & ((1_u64 << bps) - 1)) as f64)
}

// ---------------------------------------------------------------------------
// Type 4: PostScript calculator functions
// ---------------------------------------------------------------------------

#[path = "calculator_function.rs"]
mod calculator;

use calculator::{compile as compile_type4_program, execute as exec_ps_with_budget};

/// Validate PDF calculator syntax before an editing path embeds the program.
/// Runtime types, branch-dependent stack arity and arithmetic still require
/// evaluation; this syntax check is not a proof over all inputs.
pub(crate) fn validate_type4_program(program: &[u8]) -> bool {
    compile_type4_program(program).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Type 4 tokenizer / interpreter --------------------------------

    fn run_ps(program: &str, inputs: &[f64], n_outputs: usize) -> Vec<f64> {
        let (body, _) = compile_type4_program(program.as_bytes()).unwrap();
        let mut stack = calculator::initial_stack(inputs).unwrap();
        let mut remaining = MAX_FUNCTION_WORK;
        exec_ps_with_budget(&body, &mut stack, 0, &mut remaining).unwrap();
        calculator::numeric_outputs(&stack, n_outputs).unwrap()
    }

    #[test]
    fn ps_compiles_conditional_blocks_without_operand_procedures() {
        use calculator::{Instruction, Number, Op, Value};
        let (code, _) = compile_type4_program(b"{ 2 copy gt { exch } if pop }").unwrap();
        assert!(matches!(
            code[0],
            Instruction::Push(Value::Number(Number::Integer(2)))
        ));
        assert!(matches!(code[1], Instruction::Operator(Op::Copy)));
        assert!(matches!(code[2], Instruction::Operator(Op::Gt)));
        assert!(matches!(&code[3], Instruction::Branch { yes, no: None }
            if matches!(yes.as_ref(), [Instruction::Operator(Op::Exch)])));
        assert!(matches!(code[4], Instruction::Operator(Op::Pop)));
    }

    #[test]
    fn ps_basic_arithmetic() {
        assert_eq!(run_ps("{ 2 3 add }", &[], 1), vec![5.0]);
        assert_eq!(run_ps("{ 10 3 sub }", &[], 1), vec![7.0]);
        assert_eq!(run_ps("{ 4 5 mul }", &[], 1), vec![20.0]);
        assert!((run_ps("{ 9 sqrt }", &[], 1)[0] - 3.0).abs() < 1e-9);
    }

    #[test]
    fn ps_stack_exch() {
        let r = run_ps("{ 1 2 exch }", &[], 2);
        assert_eq!(r, vec![2.0, 1.0]);
    }

    #[test]
    fn ps_conditional_true_and_false() {
        assert_eq!(run_ps("{ true { 1 } { 2 } ifelse }", &[], 1), vec![1.0]);
        assert_eq!(run_ps("{ false { 1 } { 2 } ifelse }", &[], 1), vec![2.0]);
    }

    #[test]
    fn ps_if_executes_only_when_true() {
        // Keep a base value on the stack, then conditionally push 99.
        // 7 5 gt is true -> { 99 } runs -> stack [10, 99] -> top 2 outputs.
        assert_eq!(run_ps("{ 10 7 5 gt { 99 } if }", &[], 2), vec![10.0, 99.0]);
        // 5 7 gt is false -> { 99 } skipped -> stack [10] -> single output.
        assert_eq!(run_ps("{ 10 5 7 gt { 99 } if }", &[], 1), vec![10.0]);
    }

    #[test]
    fn ps_separation_tint_transform_style() {
        // A realistic Separation tint transform: input t -> CMYK linear blend.
        // { dup 0.1 mul exch 0.9 mul } on input 0.5 -> [0.05, 0.45].
        let r = run_ps("{ dup 0.1 mul exch 0.9 mul }", &[0.5], 2);
        assert!((r[0] - 0.05).abs() < 1e-9, "{:?}", r);
        assert!((r[1] - 0.45).abs() < 1e-9, "{:?}", r);
    }

    #[test]
    fn ps_roll_rotates() {
        // 1 2 3  3 1 roll -> 3 1 2
        let r = run_ps("{ 1 2 3 3 1 roll }", &[], 3);
        assert_eq!(r, vec![3.0, 1.0, 2.0]);
    }

    #[test]
    fn ps_bitshift_min_shift_does_not_panic() {
        let r = run_ps("{ 1 -2147483648 bitshift }", &[], 1);
        assert_eq!(r, vec![0.0]);
    }

    #[test]
    fn ps_index_copies_nth() {
        // 10 20 30  2 index -> 10 20 30 10
        let r = run_ps("{ 10 20 30 2 index }", &[], 4);
        assert_eq!(r, vec![10.0, 20.0, 30.0, 10.0]);
    }

    // ---- BitReader -----------------------------------------------------

    #[test]
    fn bit_reader_reads_8bit() {
        let mut br = BitReader::new(&[0xAB, 0xCD]);
        assert_eq!(br.read(8), Some(0xAB));
        assert_eq!(br.read(8), Some(0xCD));
        assert_eq!(br.read(8), None);
    }

    #[test]
    fn bit_reader_reads_16bit() {
        let mut br = BitReader::new(&[0x12, 0x34, 0xFF, 0xFF]);
        assert_eq!(br.read(16), Some(0x1234));
        assert_eq!(br.read(16), Some(0xFFFF));
    }

    #[test]
    fn bit_reader_reads_sub_byte_fields() {
        // 0b1011_0010 -> read 2,3,3 = 0b10, 0b110, 0b010
        let mut br = BitReader::new(&[0b1011_0010]);
        assert_eq!(br.read(2), Some(0b10));
        assert_eq!(br.read(3), Some(0b110));
        assert_eq!(br.read(3), Some(0b010));
    }

    #[test]
    fn read_sample_8bit() {
        let data = [0u8, 128, 255];
        assert_eq!(read_sample(&data, 0, 8), Some(0.0));
        assert_eq!(read_sample(&data, 1, 8), Some(128.0));
        assert_eq!(read_sample(&data, 2, 8), Some(255.0));
    }

    // ---- Type 0 sampled functions (end-to-end via a stream object) -----

    fn reader_for_tests() -> crate::reader::PdfReader {
        crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
    }

    fn type0_stream(
        size: &[i64],
        bps: i64,
        domain: &[f64],
        range: &[f64],
        samples: Vec<u8>,
    ) -> PdfObject {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(0));
        m.insert(
            "Size".into(),
            PdfObject::Array(size.iter().map(|&s| PdfObject::Integer(s)).collect()),
        );
        m.insert("BitsPerSample".into(), PdfObject::Integer(bps));
        m.insert(
            "Domain".into(),
            PdfObject::Array(domain.iter().map(|&v| PdfObject::Real(v)).collect()),
        );
        m.insert(
            "Range".into(),
            PdfObject::Array(range.iter().map(|&v| PdfObject::Real(v)).collect()),
        );
        m.insert("Length".into(), PdfObject::Integer(samples.len() as i64));
        PdfObject::Stream {
            dict: PdfDictionary::new(m),
            raw: samples,
        }
    }

    fn type4_stream(program: &str, range: &[f64]) -> PdfObject {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(4));
        m.insert(
            "Domain".into(),
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        m.insert(
            "Range".into(),
            PdfObject::Array(range.iter().map(|&v| PdfObject::Real(v)).collect()),
        );
        m.insert("Length".into(), PdfObject::Integer(program.len() as i64));
        PdfObject::Stream {
            dict: PdfDictionary::new(m),
            raw: program.as_bytes().to_vec(),
        }
    }

    fn type2_object(include_domain: bool) -> PdfObject {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(2));
        if include_domain {
            m.insert(
                "Domain".into(),
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            );
        }
        m.insert("C0".into(), PdfObject::Array(vec![PdfObject::Real(0.0)]));
        m.insert("C1".into(), PdfObject::Array(vec![PdfObject::Real(1.0)]));
        m.insert("N".into(), PdfObject::Real(1.0));
        PdfObject::Dictionary(PdfDictionary::new(m))
    }

    fn type2_component_object(c0: &[f64], c1: &[f64]) -> PdfObject {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(2));
        m.insert(
            "Domain".into(),
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        m.insert(
            "C0".into(),
            PdfObject::Array(c0.iter().copied().map(PdfObject::Real).collect()),
        );
        m.insert(
            "C1".into(),
            PdfObject::Array(c1.iter().copied().map(PdfObject::Real).collect()),
        );
        m.insert("N".into(), PdfObject::Real(1.0));
        PdfObject::Dictionary(PdfDictionary::new(m))
    }

    #[test]
    fn type0_1d_exact_at_sample_points() {
        // 4 samples over Domain [0,1] -> Range [0,1]: 0, 85, 170, 255 (8-bit).
        let obj = type0_stream(&[4], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0, 85, 170, 255]);
        let r = reader_for_tests();
        // At t=0 -> sample 0 -> 0.0; t=1 -> sample 3 -> 1.0.
        assert!((eval_function_n(&obj, &[0.0], &r)[0] - 0.0).abs() < 0.01);
        assert!((eval_function_n(&obj, &[1.0], &r)[0] - 1.0).abs() < 0.01);
        // t=1/3 -> sample index 1 -> 85/255 ≈ 0.333.
        let v = eval_function_n(&obj, &[1.0 / 3.0], &r)[0];
        assert!((v - 85.0 / 255.0).abs() < 0.02, "v={v}");
    }

    #[test]
    fn type0_1d_linear_interpolation_at_midpoint() {
        // 2 samples: 0 and 255 over [0,1]. Midpoint t=0.5 -> 0.5.
        let obj = type0_stream(&[2], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0, 255]);
        let r = reader_for_tests();
        let v = eval_function_n(&obj, &[0.5], &r)[0];
        assert!((v - 0.5).abs() < 0.01, "midpoint interp v={v}");
    }

    #[test]
    fn type0_2d_bilinear_interpolation() {
        // 2x2 grid, single output. Samples (dim0 fastest):
        //   (0,0)=0  (1,0)=255  (0,1)=255  (1,1)=0
        // Center (0.5,0.5) bilinear = (0+255+255+0)/4 = 127.5 -> 0.5.
        let obj = type0_stream(
            &[2, 2],
            8,
            &[0.0, 1.0, 0.0, 1.0],
            &[0.0, 1.0],
            vec![0, 255, 255, 0],
        );
        let r = reader_for_tests();
        let v = eval_function_n(&obj, &[0.5, 0.5], &r)[0];
        assert!((v - 0.5).abs() < 0.02, "bilinear center v={v}");
        // Corner (0,0) -> exactly 0.
        assert!((eval_function_n(&obj, &[0.0, 0.0], &r)[0]).abs() < 0.01);
        // Corner (1,0) -> exactly 255 -> 1.0.
        assert!((eval_function_n(&obj, &[1.0, 0.0], &r)[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn type0_rejects_missing_input_dimension_instead_of_zero_default() {
        let obj = type0_stream(
            &[2, 2],
            8,
            &[0.0, 1.0, 0.0, 1.0],
            &[0.0, 1.0],
            vec![0, 255, 255, 0],
        );
        let r = reader_for_tests();

        assert!(
            eval_function_n(&obj, &[0.5], &r).is_empty(),
            "missing second Type 0 input must not be evaluated as 0.0"
        );
    }

    #[test]
    fn type0_16bit_samples() {
        // 2 samples, 16-bit: 0x0000 and 0xFFFF over [0,1].
        let obj = type0_stream(
            &[2],
            16,
            &[0.0, 1.0],
            &[0.0, 1.0],
            vec![0x00, 0x00, 0xFF, 0xFF],
        );
        let r = reader_for_tests();
        assert!((eval_function_n(&obj, &[0.0], &r)[0]).abs() < 0.01);
        assert!((eval_function_n(&obj, &[1.0], &r)[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn type0_rejects_sample_count_cap() {
        let obj = type0_stream(
            &[MAX_TYPE0_SAMPLE_VALUES as i64 + 1],
            8,
            &[0.0, 1.0],
            &[0.0, 1.0],
            Vec::new(),
        );
        let r = reader_for_tests();
        assert!(eval_function_n(&obj, &[0.5], &r).is_empty());
    }

    #[test]
    fn type0_shape_rejects_excessive_interpolation_dimensions() {
        let dimensions = MAX_TYPE0_INTERPOLATION_DIMENSIONS + 1;
        let obj = type0_stream(
            &vec![1; dimensions],
            8,
            &vec![0.0; dimensions * 2],
            &[0.0, 1.0],
            vec![0],
        );
        let r = reader_for_tests();
        assert!(!validate_function_or_array_shape(&obj, dimensions, &r));
    }

    #[test]
    fn type0_rejects_short_sample_stream_instead_of_zero_padding() {
        let obj = type0_stream(&[2], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0]);
        let r = reader_for_tests();
        assert!(eval_function_n(&obj, &[1.0], &r).is_empty());
    }

    #[test]
    fn type0_rejects_missing_bits_per_sample_instead_of_defaulting() {
        let mut obj = type0_stream(&[2], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0, 255]);
        if let PdfObject::Stream { dict, .. } = &mut obj {
            dict.remove("BitsPerSample");
        }
        let r = reader_for_tests();
        assert!(eval_function_n(&obj, &[1.0], &r).is_empty());
    }

    #[test]
    fn type0_evaluator_rejects_malformed_local_shape_fields() {
        let r = reader_for_tests();

        let mut malformed_size = type0_stream(&[2], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0, 255]);
        if let PdfObject::Stream { dict, .. } = &mut malformed_size {
            dict.insert(
                "Size",
                PdfObject::Array(vec![PdfObject::Integer(2), PdfObject::Name("Bad".into())]),
            );
        }
        assert!(eval_function_n(&malformed_size, &[1.0], &r).is_empty());

        let mut malformed_encode = type0_stream(&[2], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0, 255]);
        if let PdfObject::Stream { dict, .. } = &mut malformed_encode {
            dict.insert(
                "Encode",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Name("Bad".into())]),
            );
        }
        assert!(eval_function_n(&malformed_encode, &[1.0], &r).is_empty());

        let mut overlong_decode = type0_stream(&[2], 8, &[0.0, 1.0], &[0.0, 1.0], vec![0, 255]);
        if let PdfObject::Stream { dict, .. } = &mut overlong_decode {
            dict.insert(
                "Decode",
                PdfObject::Array(vec![
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                ]),
            );
        }
        assert!(eval_function_n(&overlong_decode, &[1.0], &r).is_empty());
    }

    #[test]
    fn type2_rejects_missing_input_instead_of_zero_default() {
        let obj = type2_object(true);
        let r = reader_for_tests();

        assert!(
            eval_function_n(&obj, &[], &r).is_empty(),
            "missing Type 2 input must not be evaluated as 0.0"
        );
    }

    #[test]
    fn type2_rejects_missing_domain_instead_of_defaulting() {
        let obj = type2_object(false);
        let r = reader_for_tests();

        assert!(
            eval_function_n(&obj, &[0.5], &r).is_empty(),
            "missing Type 2 /Domain must not use a default domain"
        );
    }

    #[test]
    fn function_array_evaluates_one_output_component_functions() {
        let obj = PdfObject::Array(vec![
            type2_component_object(&[1.0], &[0.0]),
            type2_component_object(&[0.5], &[0.5]),
            type2_component_object(&[0.0], &[1.0]),
        ]);
        let r = reader_for_tests();

        assert!(validate_function_or_array_shape(&obj, 1, &r));
        let values = eval_function_or_array_n(&obj, &[0.25], &r);

        assert_eq!(values.len(), 3);
        assert!((values[0] - 0.75).abs() < 0.01, "R={}", values[0]);
        assert!((values[1] - 0.5).abs() < 0.01, "G={}", values[1]);
        assert!((values[2] - 0.25).abs() < 0.01, "B={}", values[2]);
    }

    #[test]
    fn function_array_rejects_multi_output_component_functions() {
        let obj = PdfObject::Array(vec![type2_component_object(&[0.0, 0.0], &[1.0, 1.0])]);
        let r = reader_for_tests();

        assert!(!validate_function_or_array_shape(&obj, 1, &r));
        assert!(
            eval_function_or_array_n(&obj, &[0.25], &r).is_empty(),
            "component arrays must not accept subfunctions with more than one output"
        );
    }

    #[test]
    fn function_array_rejects_malformed_component_functions() {
        let obj = PdfObject::Array(vec![type2_object(false)]);
        let r = reader_for_tests();

        assert!(
            !validate_function_or_array_shape(&obj, 1, &r),
            "malformed subfunction shape must fail closed before shading paint"
        );
        assert!(eval_function_or_array_n(&obj, &[0.25], &r).is_empty());
    }

    #[test]
    fn type3_rejects_missing_encode_instead_of_defaulting() {
        use std::collections::BTreeMap;
        let sub = type2_object(true);
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(3));
        m.insert(
            "Domain".into(),
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        m.insert("Functions".into(), PdfObject::Array(vec![sub]));
        m.insert("Bounds".into(), PdfObject::Array(vec![]));
        let obj = PdfObject::Dictionary(PdfDictionary::new(m));
        let r = reader_for_tests();

        assert!(
            eval_function_n(&obj, &[0.5], &r).is_empty(),
            "missing Type 3 /Encode must not use a default encode array"
        );
    }

    #[test]
    fn type4_rejects_missing_input_dimension_instead_of_empty_stack_default() {
        let obj = type4_stream("{ pop 0.5 }", &[0.0, 1.0]);
        let r = reader_for_tests();

        assert!(
            eval_function_n(&obj, &[], &r).is_empty(),
            "missing Type 4 input must not execute with an empty initial stack"
        );
        assert_eq!(eval_function_n(&obj, &[0.25], &r), vec![0.5]);
    }

    #[test]
    fn type4_rejects_boolean_or_procedure_outputs_instead_of_numeric_coercion() {
        let r = reader_for_tests();

        let bool_output = type4_stream("{ pop true }", &[0.0, 1.0]);
        assert!(
            eval_function_n(&bool_output, &[0.25], &r).is_empty(),
            "Type 4 final booleans must not be coerced to 0/1 output components"
        );

        let proc_output = type4_stream("{ 0.25 { 0.75 } }", &[0.0, 1.0]);
        assert!(
            eval_function_n(&proc_output, &[0.25], &r).is_empty(),
            "Type 4 final procedures must not be filtered while keeping earlier numeric outputs"
        );
    }

    #[test]
    fn type4_rejects_boolean_numeric_operands_and_numeric_conditions() {
        let r = reader_for_tests();

        let boolean_as_number = type4_stream("{ true 1 add }", &[0.0, 1.0]);
        assert!(
            eval_function_n(&boolean_as_number, &[0.25], &r).is_empty(),
            "Type 4 numeric operators must not coerce booleans to 0/1"
        );

        let numeric_condition = type4_stream("{ 1 { 0.5 } if }", &[0.0, 1.0]);
        assert!(
            eval_function_n(&numeric_condition, &[0.25], &r).is_empty(),
            "Type 4 if/ifelse conditions must be booleans, not numeric truthiness"
        );
    }

    #[test]
    fn function_shape_rejects_malformed_present_type2_arrays() {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(2));
        m.insert(
            "Domain".into(),
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        m.insert(
            "C0".into(),
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        m.insert(
            "C1".into(),
            PdfObject::Array(vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ]),
        );
        m.insert("N".into(), PdfObject::Real(1.0));
        let obj = PdfObject::Dictionary(PdfDictionary::new(m));
        let r = reader_for_tests();
        assert!(!validate_function_shape(&obj, 1, &r));
    }

    #[test]
    fn function_shape_allows_absent_type2_c0_c1_defaults() {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(2));
        m.insert(
            "Domain".into(),
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        m.insert("N".into(), PdfObject::Real(1.0));
        let obj = PdfObject::Dictionary(PdfDictionary::new(m));
        let r = reader_for_tests();
        assert!(validate_function_shape(&obj, 1, &r));
    }

    #[test]
    fn type4_rejects_token_cap() {
        let program = format!("{{ {} }}", "1 ".repeat(MAX_TYPE4_TOKENS + 1));
        let obj = type4_stream(&program, &[0.0, 1.0]);
        let r = reader_for_tests();
        assert!(eval_function_n(&obj, &[0.5], &r).is_empty());
    }

    #[test]
    fn type4_rejects_stack_cap() {
        let program = format!("{{ {} }}", "1 ".repeat(MAX_TYPE4_STACK + 1));
        let obj = type4_stream(&program, &[0.0, 1.0]);
        let r = reader_for_tests();
        assert!(eval_function_n(&obj, &[0.5], &r).is_empty());
    }
}
