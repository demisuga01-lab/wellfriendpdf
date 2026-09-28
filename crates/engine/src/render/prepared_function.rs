//! Immutable PDF function graphs. Preparation performs resource
//! resolution, validation, decoding and calculator parsing once. Evaluation
//! retains no reader, PDF objects or mutable cross-document cache.
use super::*;
use crate::decode_scheduler::DecodeMemoryToken;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
type ObjectId = (u32, u16);

pub(crate) struct PreparedFunction {
    roots: Vec<Arc<Node>>,
    component_array: bool,
    input_count: usize,
    /// Charged retained graph/vector/code storage, excluding builder/source
    /// temporaries and allocator overhead. Shared nodes count once per graph.
    retained_bytes: usize,
    max_decoded_stream_bytes: usize,
}

struct Node {
    domain: Vec<f64>,
    range: Option<Vec<f64>>,
    outputs: usize,
    height: usize,
    kind: Kind,
}

enum Kind {
    Sampled(Samples),
    Exponential {
        c0: Vec<f64>,
        c1: Vec<f64>,
        exponent: f64,
    },
    Stitching {
        children: Vec<Arc<Node>>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
    Calculator(Arc<[calculator::Instruction]>),
}

struct Samples {
    sizes: Vec<usize>,
    encode: Vec<f64>,
    decode: Vec<f64>,
    bits: usize,
    order: sampled::Order,
    bytes: Vec<u8>,
}

struct Builder<'a> {
    reader: &'a PdfReader,
    memo: HashMap<ObjectId, Arc<Node>>,
    active: HashSet<ObjectId>,
    visits: usize,
    bytes: usize,
    byte_limit: usize,
    resources: FunctionResources<'a>,
    memory: Option<DecodeMemoryToken>,
    max_decoded_stream_bytes: usize,
}

#[allow(dead_code)] // Convenience constructors remain for compatibility tests.
impl PreparedFunction {
    pub(crate) fn prepare_single(
        object: &PdfObject,
        inputs: usize,
        reader: &PdfReader,
    ) -> Option<Self> {
        Self::prepare_with_resources(object, inputs, false, reader, FunctionResources::default())
            .map(|(function, _)| function)
    }

    /// `inputs` is supplied by the paint consumer (two for Type 1, one for
    /// other shadings). Arrays must contain equally dimensioned scalar nodes.
    pub(crate) fn prepare(object: &PdfObject, inputs: usize, reader: &PdfReader) -> Option<Self> {
        Self::prepare_with_resources(object, inputs, true, reader, FunctionResources::default())
            .map(|(function, _)| function)
    }

    pub(super) fn prepare_with_resources(
        object: &PdfObject,
        inputs: usize,
        allow_array: bool,
        reader: &PdfReader,
        resources: FunctionResources<'_>,
    ) -> Option<(Self, Option<DecodeMemoryToken>)> {
        if inputs == 0 {
            return None;
        }
        crate::cancel::check_current_cancel("PDF function graph preparation").ok()?;
        let mut builder = Builder::new(reader, resources);
        if !allow_array {
            let root = builder.node(object, inputs, 0, 0)?;
            return builder.finish(vec![root], false, inputs);
        }
        let resolved = match object {
            PdfObject::Reference { .. } => {
                std::borrow::Cow::Owned(reader.resolve(object.clone()).ok()?)
            }
            _ => std::borrow::Cow::Borrowed(object),
        };
        let component_array = matches!(resolved.as_ref(), PdfObject::Array(_));
        let roots = if let PdfObject::Array(items) = resolved.as_ref() {
            if items.is_empty() || items.len() > MAX_FUNCTION_ARRAY_COMPONENTS {
                return None;
            }
            let mut nodes = Vec::with_capacity(items.len());
            for item in items {
                let node = builder.node(item, inputs, 0, 0)?;
                if node.outputs != 1 {
                    return None;
                }
                nodes.push(node);
            }
            nodes
        } else {
            // Retain the reference, not the temporary resolved copy: repeated
            // references and aliases in child graphs must reach the same node.
            vec![builder.node(object, inputs, 0, 0)?]
        };
        builder.finish(roots, component_array, inputs)
    }

    pub(crate) fn cached_single(
        object: &PdfObject,
        inputs: usize,
        reader: &PdfReader,
    ) -> Option<Arc<Self>> {
        super::cache::prepare(object, inputs, false, reader)
    }

    pub(crate) fn cached(
        object: &PdfObject,
        inputs: usize,
        reader: &PdfReader,
    ) -> Option<Arc<Self>> {
        super::cache::prepare(object, inputs, true, reader)
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(crate) fn cached_with_resources(
        object: &PdfObject,
        inputs: usize,
        allow_array: bool,
        reader: &PdfReader,
        resources: FunctionResources<'_>,
    ) -> Option<FunctionLease> {
        super::cache::prepare_with_resources(object, inputs, allow_array, reader, resources)
    }

    pub(super) fn accepts_resources(&self, resources: FunctionResources<'_>) -> bool {
        self.retained_bytes <= resources.max_graph_bytes
            && self.max_decoded_stream_bytes <= resources.max_stream_bytes
    }

    /// Declared arity is stable after graph validation; scalar consumers must
    /// not silently discard additional channels.
    pub(crate) fn output_count(&self) -> usize {
        if self.component_array {
            self.roots.len()
        } else {
            self.roots[0].outputs
        }
    }

    pub(crate) fn evaluate(&self, inputs: &[f64]) -> Vec<f64> {
        self.evaluate_metered(inputs, MAX_FUNCTION_WORK).0
    }

    /// Shading consumers debit actual scalar/tap/stack work to their existing
    /// cumulative render budget. A caller allowance cannot increase our cap.
    pub(crate) fn evaluate_metered(&self, inputs: &[f64], allowance: usize) -> (Vec<f64>, usize) {
        let initial = allowance.min(MAX_FUNCTION_WORK);
        let mut remaining = initial;
        let values = self
            .evaluate_with_budget(inputs, &mut remaining)
            .unwrap_or_default();
        (values, initial - remaining)
    }

    fn evaluate_with_budget(&self, inputs: &[f64], remaining: &mut usize) -> Option<Vec<f64>> {
        if inputs.len() != self.input_count || !inputs.iter().all(|v| v.is_finite()) {
            return None;
        }
        charge_work(remaining, inputs.len())?;
        if !self.component_array {
            return self.roots[0].evaluate(inputs, remaining);
        }
        let mut values = Vec::with_capacity(self.roots.len());
        for root in &self.roots {
            let output = root.evaluate(inputs, remaining)?;
            values.push(*output.first()?);
        }
        Some(values)
    }
}

impl<'a> Builder<'a> {
    fn new(reader: &'a PdfReader, resources: FunctionResources<'a>) -> Self {
        let byte_limit = MAX_RETAINED_BYTES.min(resources.max_graph_bytes);
        Self {
            reader,
            memo: HashMap::new(),
            active: HashSet::new(),
            visits: MAX_FUNCTION_VISITS,
            bytes: byte_limit,
            byte_limit,
            resources,
            memory: resources.memory.map(|budget| budget.reservation()),
            max_decoded_stream_bytes: 0,
        }
    }

    fn finish(
        mut self,
        roots: Vec<Arc<Node>>,
        component_array: bool,
        input_count: usize,
    ) -> Option<(PreparedFunction, Option<DecodeMemoryToken>)> {
        self.retain(std::mem::size_of::<PreparedFunction>() + 2 * std::mem::size_of::<usize>())?;
        self.retain(
            roots
                .capacity()
                .checked_mul(std::mem::size_of::<Arc<Node>>())?,
        )?;
        Some((
            PreparedFunction {
                roots,
                component_array,
                input_count,
                retained_bytes: self.byte_limit - self.bytes,
                max_decoded_stream_bytes: self.max_decoded_stream_bytes,
            },
            self.memory,
        ))
    }

    fn retain(&mut self, bytes: usize) -> Option<()> {
        let remaining = self.bytes.checked_sub(bytes)?;
        if let Some(memory) = &mut self.memory {
            memory
                .resize(memory.bytes().checked_add(u64::try_from(bytes).ok()?)?)
                .ok()?;
        }
        self.bytes = remaining;
        Some(())
    }

    fn decode(
        &mut self,
        object: &PdfObject,
        minimum: usize,
        maximum: usize,
    ) -> Option<(Vec<u8>, Option<DecodeMemoryToken>)> {
        let maximum = maximum.min(self.resources.max_stream_bytes).min(self.bytes);
        if minimum > maximum {
            return None;
        }
        let mut window = self
            .resources
            .memory
            .map(|budget| budget.try_acquire_up_to(minimum as u64, maximum as u64))
            .transpose()
            .ok()?;
        let maximum = window
            .as_ref()
            .map_or(maximum, |token| token.bytes() as usize);
        let bytes = resolve_stream_bytes_limited(object, self.reader, Some(maximum))?;
        if bytes.len() < minimum || bytes.capacity() > self.bytes {
            return None;
        }
        if let Some(window) = &mut window {
            window.resize(bytes.capacity() as u64).ok()?;
        }
        self.max_decoded_stream_bytes = self.max_decoded_stream_bytes.max(bytes.len());
        Some((bytes, window))
    }

    fn retain_decoded(&mut self, bytes: usize, window: Option<DecodeMemoryToken>) -> Option<()> {
        if let Some(window) = window {
            let remaining = self.bytes.checked_sub(bytes)?;
            self.memory.as_mut()?.absorb(window).ok()?;
            self.bytes = remaining;
            Some(())
        } else {
            self.retain(bytes)
        }
    }

    fn node(
        &mut self,
        object: &PdfObject,
        inputs: usize,
        depth: usize,
        aliases: usize,
    ) -> Option<Arc<Node>> {
        if depth > 16 || aliases > 64 {
            return None;
        }
        // Resolve aliases iteratively: 64 aliases at each stitching level
        // must not multiply native call-stack depth by the function depth.
        let mut resolved = std::borrow::Cow::Borrowed(object);
        let mut ids = Vec::new();
        let result = (|| {
            loop {
                charge_work(&mut self.visits, 1)?;
                let Some(id) = resolved.as_reference() else {
                    break;
                };
                if aliases + ids.len() >= 64 {
                    return None;
                }
                if let Some(node) = self.memo.get(&id) {
                    return (node.domain.len() / 2 == inputs && depth + node.height <= 16)
                        .then(|| Arc::clone(node));
                }
                if !self.active.insert(id) {
                    return None;
                }
                ids.push(id);
                resolved = std::borrow::Cow::Owned(self.reader.get_object(id.0, id.1).ok()?);
            }
            self.direct_node(&resolved, inputs, depth)
        })();
        for id in ids {
            self.active.remove(&id);
            if let Some(node) = &result {
                self.memo.insert(id, Arc::clone(node));
            }
        }
        result
    }

    fn direct_node(
        &mut self,
        object: &PdfObject,
        inputs: usize,
        depth: usize,
    ) -> Option<Arc<Node>> {
        let source = match object {
            PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict,
            _ => return None,
        };
        // Borrow already-direct metadata. Cloning a direct stitching dictionary
        // at every level would also clone all descendant stream payloads.
        let dict = super::super::parameter_dictionary::function(source, Some(self.reader)).ok()?;
        let domain = ordered_pairs(&dict, "Domain", true)??;
        if domain.len() != inputs.checked_mul(2)? {
            return None;
        }
        let range = ordered_pairs(&dict, "Range", false)?;
        self.retain(std::mem::size_of::<Node>() + 2 * std::mem::size_of::<usize>())?;
        self.retain(
            domain
                .capacity()
                .checked_add(range.as_ref().map_or(0, Vec::capacity))?
                .checked_mul(8)?,
        )?;
        let (kind, outputs) = match dict.get_integer("FunctionType")? {
            0 => {
                let outputs = validate_type0_shape(&dict)?;
                let sizes = strict_type0_size(&dict)?;
                let order = strict_type0_order(&dict)?;
                let bits = usize::try_from(dict.get_integer("BitsPerSample")?).ok()?;
                let encode = strict_float_array_field(&dict, "Encode")
                    .ok()?
                    .unwrap_or_else(|| {
                        sizes
                            .iter()
                            .flat_map(|&size| [0.0, (size - 1) as f64])
                            .collect()
                    });
                let decode = strict_float_array_field(&dict, "Decode")
                    .ok()?
                    .unwrap_or_else(|| range.as_ref().unwrap().clone());
                self.retain(sizes.capacity().checked_mul(std::mem::size_of::<usize>())?)?;
                self.retain(
                    encode
                        .capacity()
                        .checked_add(decode.capacity())?
                        .checked_mul(8)?,
                )?;
                let needed = sizes
                    .iter()
                    .try_fold(outputs, |n, &size| n.checked_mul(size))?
                    .checked_mul(bits)?
                    .div_ceil(8);
                if needed > self.bytes {
                    return None;
                }
                let (bytes, window) = self.decode(object, needed, MAX_TYPE0_SAMPLE_VALUES * 4)?;
                // Charge capacity: decoded buffers can reserve more than their
                // final length. Shared referenced nodes are charged only once.
                self.retain_decoded(bytes.capacity(), window)?;
                (
                    Kind::Sampled(Samples {
                        sizes,
                        encode,
                        decode,
                        bits,
                        order,
                        bytes,
                    }),
                    outputs,
                )
            }
            2 => {
                let outputs = validate_type2_shape(&dict)?;
                let c0 = strict_float_array_field(&dict, "C0")
                    .ok()?
                    .unwrap_or_else(|| vec![0.0]);
                let c1 = strict_float_array_field(&dict, "C1")
                    .ok()?
                    .unwrap_or_else(|| vec![1.0]);
                self.retain(c0.capacity().checked_add(c1.capacity())?.checked_mul(8)?)?;
                (
                    Kind::Exponential {
                        c0,
                        c1,
                        exponent: dict.get("N")?.as_number()?,
                    },
                    outputs,
                )
            }
            3 => {
                if inputs != 1 {
                    return None;
                }
                let objects = dict.get("Functions")?.as_array()?;
                if objects.is_empty()
                    || objects.len() > super::super::parameter_dictionary::MAX_STITCHING_FUNCTIONS
                    || (objects.len() > 1 && domain[0] >= domain[1])
                {
                    return None;
                }
                let bounds = require_strict_float_array_exact(&dict, "Bounds", objects.len() - 1)?;
                let encode = require_strict_float_array_exact(
                    &dict,
                    "Encode",
                    objects.len().checked_mul(2)?,
                )?;
                let mut previous = domain[0];
                for &bound in &bounds {
                    if bound <= previous || bound > domain[1] {
                        return None;
                    }
                    previous = bound;
                }
                self.retain(
                    bounds
                        .capacity()
                        .checked_add(encode.capacity())?
                        .checked_mul(8)?,
                )?;
                self.retain(
                    objects
                        .len()
                        .checked_mul(std::mem::size_of::<Arc<Node>>())?,
                )?;
                let mut children: Vec<Arc<Node>> = Vec::with_capacity(objects.len());
                for object in objects {
                    let child = self.node(object, 1, depth + 1, 0)?;
                    if children
                        .first()
                        .is_some_and(|first| first.outputs != child.outputs)
                    {
                        return None;
                    }
                    children.push(child);
                }
                let outputs = children[0].outputs;
                (
                    Kind::Stitching {
                        children,
                        bounds,
                        encode,
                    },
                    outputs,
                )
            }
            4 => {
                let outputs = validate_type4_shape(&dict, inputs)?;
                if inputs > MAX_TYPE4_STACK || outputs > MAX_TYPE4_STACK {
                    return None;
                }
                let (bytes, _window) = self.decode(object, 1, MAX_TYPE4_PROGRAM_BYTES)?;
                let (program, storage) = compile_type4_program(&bytes)?;
                self.retain(storage)?;
                (Kind::Calculator(program), outputs)
            }
            _ => return None,
        };
        if range
            .as_ref()
            .is_some_and(|values| values.len() != outputs * 2)
        {
            return None;
        }
        let height = match &kind {
            Kind::Stitching { children, .. } => {
                1 + children.iter().map(|child| child.height).max()?
            }
            _ => 0,
        };
        if depth + height > 16 {
            return None;
        }
        Some(Arc::new(Node {
            domain,
            range,
            outputs,
            height,
            kind,
        }))
    }
}

impl Node {
    fn evaluate(&self, inputs: &[f64], remaining: &mut usize) -> Option<Vec<f64>> {
        charge_work(remaining, 1 + inputs.len() + self.outputs)?;
        if inputs.len() * 2 != self.domain.len() || !inputs.iter().all(|v| v.is_finite()) {
            return None;
        }
        let clipped: std::borrow::Cow<'_, [f64]> = if inputs
            .iter()
            .enumerate()
            .all(|(i, &value)| value >= self.domain[2 * i] && value <= self.domain[2 * i + 1])
        {
            std::borrow::Cow::Borrowed(inputs)
        } else {
            std::borrow::Cow::Owned(
                inputs
                    .iter()
                    .enumerate()
                    .map(|(i, &value)| value.clamp(self.domain[2 * i], self.domain[2 * i + 1]))
                    .collect(),
            )
        };
        let mut output = match &self.kind {
            Kind::Sampled(samples) => {
                let mut coordinates = [0.0; MAX_TYPE0_INTERPOLATION_DIMENSIONS];
                for (i, &value) in clipped.iter().enumerate() {
                    let position =
                        domain_position(value, self.domain[2 * i], self.domain[2 * i + 1])?;
                    coordinates[i] = crate::render::shading::shading_domain_value(
                        samples.encode[2 * i],
                        samples.encode[2 * i + 1],
                        position,
                    )
                    .clamp(0.0, (samples.sizes[i] - 1) as f64);
                }
                let mut values = sampled::interpolate_with_budget(
                    &samples.bytes,
                    &samples.sizes,
                    &coordinates[..samples.sizes.len()],
                    self.outputs,
                    samples.bits,
                    samples.order,
                    remaining,
                )?;
                for (i, value) in values.iter_mut().enumerate() {
                    *value = crate::render::shading::shading_domain_value(
                        samples.decode[2 * i],
                        samples.decode[2 * i + 1],
                        *value,
                    );
                    // Sample decoding can overflow an extreme finite Decode;
                    // the declared finite Range clips it before finite checks.
                    let range = self.range.as_ref()?;
                    *value = value.clamp(range[2 * i], range[2 * i + 1]);
                }
                values
            }
            Kind::Exponential { c0, c1, exponent } => {
                crate::render::shading::exponential_components(c0, c1, *exponent, clipped[0])?
            }
            Kind::Stitching {
                children,
                bounds,
                encode,
            } => {
                let x = clipped[0];
                let index = bounds.partition_point(|&bound| x >= bound);
                let lo = if index == 0 {
                    self.domain[0]
                } else {
                    bounds[index - 1]
                };
                let hi = bounds.get(index).copied().unwrap_or(self.domain[1]);
                let position = domain_position(x, lo, hi)?;
                let mapped = crate::render::shading::shading_domain_value(
                    encode[2 * index],
                    encode[2 * index + 1],
                    position,
                );
                children[index].evaluate(&[mapped], remaining)?
            }
            Kind::Calculator(program) => {
                let mut stack = calculator::initial_stack(&clipped)?;
                exec_ps_with_budget(program, &mut stack, 0, remaining).ok()?;
                calculator::numeric_outputs(&stack, self.outputs)?
            }
        };
        if output.len() != self.outputs || !output.iter().all(|v| v.is_finite()) {
            return None;
        }
        if let Some(range) = &self.range {
            for (i, value) in output.iter_mut().enumerate() {
                *value = value.clamp(range[2 * i], range[2 * i + 1]);
            }
        }
        Some(output)
    }
}

#[cfg(test)]
#[path = "prepared_function_tests.rs"]
mod tests;
