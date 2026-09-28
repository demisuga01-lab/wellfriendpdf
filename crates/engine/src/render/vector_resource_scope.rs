//! Private bindings for graphics-state resources inherited across vector scopes.
//!
//! Aliases exist only in the interpreter's resource model. They never rename
//! source operands, and cannot be selected by the child program: all names in
//! that program and its explicit resources are reserved before allocating them.
use std::collections::BTreeSet;

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{ColorSpace, GraphicsState};
use crate::engine::PageResources;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

fn invalid(message: &str) -> WellfriendError {
    WellfriendError::MalformedPdf(format!("inherited vector resource: {message}"))
}

struct Aliases {
    reserved: BTreeSet<String>,
    next: usize,
}

impl Aliases {
    fn new(
        resources: &PageResources,
        ops: &[ContentOperation],
        reader: &PdfReader,
    ) -> Result<Self> {
        let mut reserved = BTreeSet::new();
        for name in resources
            .fonts
            .keys()
            .chain(resources.color_spaces.keys())
            .chain(resources.patterns.keys())
            .chain(resources.ext_g_states.keys())
            .chain(resources.xobjects.keys())
            .chain(resources.shadings.keys())
            .chain(resources.properties.keys())
        {
            reserved.insert(name.clone());
        }
        // ExtGState's normalized Font operand and named colour/pattern
        // dependencies can refer to names without a direct content operand.
        fn reserve_object_names(
            object: &PdfObject,
            names: &mut BTreeSet<String>,
            depth: usize,
            budget: &mut usize,
            visited: &mut BTreeSet<(u32, u16)>,
            reader: &PdfReader,
        ) -> Result<()> {
            if depth > 64 {
                return Err(WellfriendError::ResourceLimit(
                    "vector resource name depth".into(),
                ));
            }
            *budget = budget.checked_sub(1).ok_or_else(|| {
                WellfriendError::ResourceLimit("vector resource name budget".into())
            })?;
            match object {
                PdfObject::Reference { number, generation }
                    if visited.insert((*number, *generation)) =>
                {
                    // A missing unused resource is not an alias collision;
                    // its normal use-site resolution still fails if used.
                    if let Ok(value) = reader.get_object(*number, *generation) {
                        reserve_object_names(&value, names, depth + 1, budget, visited, reader)?;
                    }
                }
                PdfObject::Name(name) => {
                    names.insert(name.clone());
                }
                PdfObject::Array(values) => {
                    for value in values {
                        reserve_object_names(value, names, depth + 1, budget, visited, reader)?;
                    }
                }
                PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
                    for (key, value) in dict.entries() {
                        names.insert(key.clone());
                        // Font program and CMap bytes do not resolve graphics
                        // resource names. Do not load large unrelated streams.
                        if !matches!(
                            key.as_str(),
                            "FontFile" | "FontFile2" | "FontFile3" | "ToUnicode"
                        ) {
                            reserve_object_names(value, names, depth + 1, budget, visited, reader)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let mut budget = 1_000_000usize;
        let mut visited = BTreeSet::new();
        for dict in resources
            .fonts
            .values()
            .chain(resources.ext_g_states.values())
        {
            for (key, value) in dict.entries() {
                reserved.insert(key.clone());
                if !matches!(
                    key.as_str(),
                    "FontFile" | "FontFile2" | "FontFile3" | "ToUnicode"
                ) {
                    reserve_object_names(
                        value,
                        &mut reserved,
                        0,
                        &mut budget,
                        &mut visited,
                        reader,
                    )?;
                }
            }
        }
        for object in resources
            .color_spaces
            .values()
            .chain(resources.patterns.values())
            .chain(resources.shadings.values())
            .chain(resources.properties.values())
        {
            reserve_object_names(object, &mut reserved, 0, &mut budget, &mut visited, reader)?;
        }
        for op in ops {
            if op.operands.len() > budget {
                return Err(WellfriendError::ResourceLimit(
                    "vector resource operand scan budget".into(),
                ));
            }
            let mut pending = op.operands.iter().collect::<Vec<_>>();
            while let Some(operand) = pending.pop() {
                budget = budget.checked_sub(1).ok_or_else(|| {
                    WellfriendError::ResourceLimit("vector resource operand scan budget".into())
                })?;
                match operand {
                    Operand::Name(name) => {
                        reserved.insert(name.clone());
                    }
                    Operand::Array(values) => {
                        if values.len().saturating_add(pending.len()) > budget {
                            return Err(WellfriendError::ResourceLimit(
                                "vector resource operand scan budget".into(),
                            ));
                        }
                        pending.extend(values);
                    }
                    Operand::Dictionary(entries) => {
                        if entries.len().saturating_add(pending.len()) > budget {
                            return Err(WellfriendError::ResourceLimit(
                                "vector resource operand scan budget".into(),
                            ));
                        }
                        for (name, value) in entries {
                            reserved.insert(name.clone());
                            pending.push(value);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Self { reserved, next: 0 })
    }

    fn allocate(&mut self) -> String {
        loop {
            let name = format!("WFInheritedVector{}", self.next);
            self.next += 1;
            if self.reserved.insert(name.clone()) {
                return name;
            }
        }
    }
}

/// Resolve named colour dependencies in their selection scope before carrying
/// the selected object into a different scope. Do not borrow the child's names.
fn bound_color_space(
    object: &PdfObject,
    source: &PageResources,
    reader: &PdfReader,
    depth: usize,
) -> Result<PdfObject> {
    if depth > 16 {
        return Err(invalid("cyclic or excessively deep colour space"));
    }
    match reader.resolve(object.clone())? {
        PdfObject::Name(name)
            if !matches!(
                name.as_str(),
                "DeviceGray" | "G" | "DeviceRGB" | "RGB" | "DeviceCMYK" | "CMYK" | "Pattern"
            ) =>
        {
            let value = source
                .color_spaces
                .get(&name)
                .ok_or_else(|| invalid("missing selected colour space"))?;
            bound_color_space(value, source, reader, depth + 1)
        }
        PdfObject::Array(mut values) => {
            let base = match values.first().and_then(PdfObject::as_name) {
                Some("Indexed" | "I" | "Pattern") => Some(1),
                Some("Separation" | "DeviceN") => Some(2),
                _ => None,
            };
            if let Some(index) = base {
                if let Some(value) = values.get(index) {
                    values[index] = bound_color_space(value, source, reader, depth + 1)?;
                }
            }
            Ok(PdfObject::Array(values))
        }
        other => Ok(other),
    }
}

fn bind_color(
    space: &mut ColorSpace,
    source: &PageResources,
    target: &mut PageResources,
    aliases: &mut Aliases,
    reader: &PdfReader,
) -> Result<()> {
    let ColorSpace::Named(name) = space else {
        return Ok(());
    };
    if name == "Pattern" {
        return Ok(());
    }
    let object = source
        .color_spaces
        .get(name)
        .ok_or_else(|| invalid("missing selected colour space"))?;
    let object = bound_color_space(object, source, reader, 0)?;
    let alias = aliases.allocate();
    target.color_spaces.insert(alias.clone(), object);
    *name = alias;
    Ok(())
}

fn bound_pattern(
    original: &PdfObject,
    source: &PageResources,
    reader: &PdfReader,
) -> Result<PdfObject> {
    let mut object = reader.resolve(original.clone())?;
    let dict = match &mut object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict,
        _ => return Err(invalid("selected pattern is not a dictionary")),
    };
    if dict.get_integer("PatternType") == Some(2) {
        let mut shading = reader.resolve(
            dict.get("Shading")
                .cloned()
                .ok_or_else(|| invalid("missing shading pattern"))?,
        )?;
        let shading_dict: &mut PdfDictionary = match &mut shading {
            PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict,
            _ => return Err(invalid("shading pattern is not a dictionary")),
        };
        if let Some(space) = shading_dict.get("ColorSpace") {
            let space = bound_color_space(space, source, reader, 0)?;
            shading_dict.insert("ColorSpace", space);
        }
        dict.insert("Shading", shading);
    } else {
        // Tiling streams own their dictionary; retain their indirect identity
        // instead of storing another copy of a potentially large raw stream.
        return Ok(original.clone());
    }
    Ok(object)
}

pub(crate) fn bind_inherited_state(
    target: &mut PageResources,
    source: &PageResources,
    state: &mut GraphicsState,
    ops: &[ContentOperation],
    reader: &PdfReader,
) -> Result<()> {
    let named_space =
        |space: &ColorSpace| matches!(space, ColorSpace::Named(name) if name != "Pattern");
    if state.text.font_name.is_empty()
        && !named_space(&state.fill_color_space)
        && !named_space(&state.stroke_color_space)
        && !named_space(&state.fill_color.space)
        && !named_space(&state.stroke_color.space)
        && state.fill_pattern_name.is_none()
        && state.stroke_pattern_name.is_none()
    {
        // Most vector-only Forms inherit only device colours and numeric
        // graphics state. No private name is needed, so avoid a graph scan.
        return Ok(());
    }
    let mut aliases = Aliases::new(target, ops, reader)?;
    if !state.text.font_name.is_empty() {
        let name = &state.text.font_name;
        let alias = aliases.allocate();
        if let Some(font) = source.fonts.get(name) {
            target.fonts.insert(alias.clone(), font.clone());
            if let Some(reference) = source.font_references.get(name) {
                target.font_references.insert(alias.clone(), *reference);
            }
        }
        // Preserve a missing selection as missing; a matching name in the child
        // must not accidentally repair/rebind an invalid inherited selection.
        state.text.font_name = alias;
    }
    bind_color(
        &mut state.fill_color_space,
        source,
        target,
        &mut aliases,
        reader,
    )?;
    bind_color(
        &mut state.stroke_color_space,
        source,
        target,
        &mut aliases,
        reader,
    )?;
    bind_color(
        &mut state.fill_color.space,
        source,
        target,
        &mut aliases,
        reader,
    )?;
    bind_color(
        &mut state.stroke_color.space,
        source,
        target,
        &mut aliases,
        reader,
    )?;
    for name in [&mut state.fill_pattern_name, &mut state.stroke_pattern_name]
        .into_iter()
        .flatten()
    {
        let object = source
            .patterns
            .get(name)
            .ok_or_else(|| invalid("missing selected pattern"))?;
        let alias = aliases.allocate();
        target
            .patterns
            .insert(alias.clone(), bound_pattern(object, source, reader)?);
        *name = alias;
    }
    Ok(())
}

#[cfg(test)]
#[path = "vector_resource_scope_tests.rs"]
mod tests;
