//! Bounded structural inspection of TrueType programs. This is not a hint VM or
//! a proof of dynamic stack/jump behavior; immediate data is never scanned as code.
use crate::{Result, WellfriendError};
use std::ops::Range;

pub(super) const PROGRAM_LIMIT: usize = 4 * 1024 * 1024;
pub(super) fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("TrueType instructions: {message}"))
}
#[derive(Default)]
pub(super) struct Budget {
    bytes: usize,
    steps: usize,
    events: usize,
}
impl Budget {
    fn source(&mut self, length: usize) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(length)
            .ok_or_else(|| fail("source size overflow"))?;
        if length > PROGRAM_LIMIT || self.bytes > 64 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "TrueType program byte budget exceeded".into(),
            ));
        }
        Ok(())
    }
    fn step(&mut self) -> Result<()> {
        self.steps += 1;
        if self.steps > 4_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "TrueType instruction scan exceeds four million operations".into(),
            ));
        }
        if self.steps.is_multiple_of(256) {
            crate::cancel::check_current_cancel("TrueType program inspection")?;
        }
        Ok(())
    }
    fn event(&mut self) -> Result<()> {
        self.events += 1;
        if self.events > 262144 {
            return Err(WellfriendError::ResourceLimit(
                "TrueType hint receipts exceed 262144".into(),
            ));
        }
        Ok(())
    }
    pub fn work(&self) -> usize {
        self.steps
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Font,
    Preparation,
    Glyph(u16),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryKind {
    Variation,
    Information,
    LegacyData,
}
#[derive(Debug)]
pub(crate) struct Query {
    pub offset: usize,
    pub kind: QueryKind,
    pub in_definition: bool,
    /// A literal immediately preceding GETINFO, not inferred through calls.
    pub information_selector: Option<i32>,
}
#[derive(Debug)]
pub(crate) struct Definition {
    pub offset: usize,
    pub body: Range<usize>,
    pub instruction: bool,
    /// Some only for an immediately preceding nonempty literal push.
    pub identifier: Option<i32>,
}
#[derive(Debug)]
pub(crate) struct Inventory {
    pub owner: Owner,
    #[allow(dead_code)] // Audit accounting is consumed by bytecode tests and reports.
    pub source_bytes: usize,
    pub instruction_count: usize,
    pub maximum_literal_push: usize,
    pub queries: Vec<Query>,
    pub definitions: Vec<Definition>,
    pub top_level_calls: usize,
    pub top_level_jumps: usize,
    pub relative_jumps: usize,
    pub unknown_opcodes: usize,
}
#[derive(Debug)]
pub(super) struct Instruction {
    pub range: Range<usize>,
    pub opcode: u8,
    pub push_count: usize,
    pub last_literal: Option<i32>,
}
pub(super) fn decode(data: &[u8], at: usize) -> Result<Instruction> {
    let opcode = *data
        .get(at)
        .ok_or_else(|| fail("instruction outside source"))?;
    let mut next = at + 1;
    let (count, size) = match opcode {
        0x40 | 0x41 => {
            let count = usize::from(*data.get(next).ok_or_else(|| fail("missing push count"))?);
            next += 1;
            (count, if opcode == 0x40 { 1 } else { 2 })
        }
        0xb0..=0xb7 => (usize::from(opcode - 0xb0 + 1), 1),
        0xb8..=0xbf => (usize::from(opcode - 0xb8 + 1), 2),
        _ => (0, 1),
    };
    let end = next
        .checked_add(count * size)
        .ok_or_else(|| fail("push extent overflow"))?;
    let values = data
        .get(next..end)
        .ok_or_else(|| fail("truncated push immediate data"))?;
    let last_literal = if count == 0 {
        None
    } else if size == 1 {
        Some(i32::from(values[values.len() - 1]))
    } else {
        Some(i32::from(i16::from_be_bytes(
            values[values.len() - 2..].try_into().unwrap(),
        )))
    };
    Ok(Instruction {
        range: at..end,
        opcode,
        push_count: count,
        last_literal,
    })
}
pub(super) fn scan(data: &[u8], owner: Owner, budget: &mut Budget) -> Result<Inventory> {
    crate::cancel::check_current_cancel("TrueType program inspection")?;
    budget.source(data.len())?;
    let mut inventory = Inventory {
        owner,
        source_bytes: data.len(),
        instruction_count: 0,
        maximum_literal_push: 0,
        queries: Vec::new(),
        definitions: Vec::new(),
        top_level_calls: 0,
        top_level_jumps: 0,
        relative_jumps: 0,
        unknown_opcodes: 0,
    };
    let mut at = 0;
    let mut previous_literal = None;
    // Definitions can live inside an outer IF, but their own conditionals must
    // balance locally before ENDF. Definitions cannot nest in one another.
    let mut definition = None::<(Definition, usize)>;
    let mut conditionals = Vec::<bool>::new();
    while at < data.len() {
        budget.step()?;
        let ins = decode(data, at)?;
        inventory.instruction_count += 1;
        inventory.maximum_literal_push = inventory.maximum_literal_push.max(ins.push_count);
        match ins.opcode {
            0x2c | 0x89 => {
                if matches!(owner, Owner::Glyph(_)) {
                    return Err(fail("glyph programs cannot define functions/instructions"));
                }
                if definition.is_some() {
                    return Err(fail("nested instruction/function definition"));
                }
                budget.event()?;
                let instruction = ins.opcode == 0x89;
                if instruction && previous_literal.is_some_and(|id| !(0..=255).contains(&id)) {
                    return Err(fail("literal IDEF opcode outside byte range"));
                }
                definition = Some((
                    Definition {
                        offset: at,
                        body: ins.range.end..ins.range.end,
                        instruction,
                        identifier: previous_literal,
                    },
                    conditionals.len(),
                ));
            }
            0x2d => {
                let (mut current, outer_depth) = definition
                    .take()
                    .ok_or_else(|| fail("ENDF outside a definition"))?;
                if conditionals.len() != outer_depth {
                    return Err(fail("unclosed conditional in definition"));
                }
                current.body.end = at;
                if current.body.len() >= 65536 {
                    return Err(fail("definition exceeds 64 KiB"));
                }
                inventory.definitions.push(current);
            }
            0x58 => {
                if conditionals.len() >= 256 {
                    return Err(WellfriendError::ResourceLimit(
                        "TrueType conditional depth exceeds 256".into(),
                    ));
                }
                conditionals.push(false);
            }
            0x1b | 0x59 => {
                let floor = definition.as_ref().map_or(0, |(_, depth)| *depth);
                if conditionals.len() <= floor {
                    return Err(fail("conditional crosses its definition/program owner"));
                }
                if ins.opcode == 0x59 {
                    conditionals.pop();
                } else {
                    let seen_else = conditionals.last_mut().unwrap();
                    if *seen_else {
                        return Err(fail("duplicate ELSE"));
                    }
                    *seen_else = true;
                }
            }
            0x88 | 0x91 | 0x92 => {
                budget.event()?;
                inventory.queries.push(Query {
                    offset: at,
                    kind: match ins.opcode {
                        0x88 => QueryKind::Information,
                        0x91 => QueryKind::Variation,
                        _ => QueryKind::LegacyData,
                    },
                    in_definition: definition.is_some(),
                    information_selector: (ins.opcode == 0x88)
                        .then_some(previous_literal)
                        .flatten(),
                });
            }
            0x2a | 0x2b if definition.is_none() => {
                inventory.top_level_calls += 1;
            }
            0x1c | 0x78 | 0x79 => {
                inventory.relative_jumps += 1;
                if definition.is_none() {
                    inventory.top_level_jumps += 1;
                }
            }
            0x28 | 0x7b | 0x83 | 0x84 | 0x8f | 0x90 | 0x93..=0xaf => {
                inventory.unknown_opcodes += 1;
            }
            _ => {}
        }
        // A definition's body is not executed when it is installed. In
        // particular, its final PUSH is not a literal on the enclosing stack.
        previous_literal = ins.last_literal;
        at = ins.range.end;
    }
    if definition.is_some() || !conditionals.is_empty() {
        return Err(fail("unclosed definition/conditional"));
    }
    crate::cancel::check_current_cancel("TrueType inventory publication")?;
    Ok(inventory)
}

#[cfg(test)]
#[path = "tt_bytecode_tests.rs"]
mod tests;
