//! Cell-local block flow. Paragraph byte ranges, shaping, bidi and styles stay
//! separate; the virtual cell cursor only orders fragmentation. An empty block
//! consumes a layout item so blank paragraphs cannot disappear or stall flow.
use super::*;

pub(super) struct CellBlock {
    pub paragraph: usize,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone)]
pub(super) struct FlowLine {
    pub paragraph: usize,
    pub bytes: std::ops::Range<usize>,
    pub cell_start: usize,
    pub cell_end: usize,
    pub metric: LineMetrics,
    pub before: f64,
    pub after: f64,
    pub advance: f64,
    pub empty: bool,
}
impl FlowLine {
    pub fn height(&self) -> f64 {
        self.before + self.advance + self.after
    }
}
#[derive(Clone)]
pub(super) struct CachedFlow {
    lines: std::sync::Arc<[FlowLine]>,
    prefix: std::sync::Arc<[f64]>,
}

pub(super) fn prepare_blocks(
    request: &LinkedStoryRequest,
) -> Result<(Vec<Vec<CellBlock>>, Vec<usize>)> {
    let by_id = request
        .paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect::<BTreeMap<_, _>>();
    let mut cells = Vec::new();
    let mut lengths = Vec::new();
    for cell in &request.table_layout.as_ref().unwrap().cells {
        let mut blocks = Vec::new();
        let mut start = 0usize;
        for id in cell.block_ids() {
            let paragraph = *by_id
                .get(id)
                .ok_or_else(|| fail("table block paragraph missing"))?;
            let end = start
                .checked_add(request.paragraphs[paragraph].text.len())
                .and_then(|n| n.checked_add(usize::from(!cell.paragraph_ids.is_empty())))
                .ok_or_else(|| fail("table block cursor overflow"))?;
            blocks.push(CellBlock {
                paragraph,
                start,
                end,
            });
            start = end;
        }
        cells.push(blocks);
        lengths.push(start);
    }
    Ok((cells, lengths))
}

impl Prepared<'_> {
    pub(super) fn flow_lines(
        &self,
        cell: usize,
        from: usize,
        started: bool,
        width: f64,
    ) -> Result<(LineSlice<FlowLine>, f64)> {
        if from > self.cell_lengths[cell] {
            return Err(fail("invalid cell block cursor"));
        }
        if started && from == self.cell_lengths[cell] {
            return Ok((
                LineSlice {
                    data: Vec::new().into(),
                    start: 0,
                },
                0.0,
            ));
        }
        let key = (cell, width.to_bits());
        let found = self.flows.borrow().get(&key).and_then(|cached| {
            let start = cached
                .lines
                .binary_search_by_key(&from, |l| l.cell_start)
                .ok()?;
            Some((cached.clone(), start))
        });
        let (cache, start) = if let Some(found) = found {
            found
        } else {
            let mut lines = Vec::new();
            for block in &self.cell_blocks[cell] {
                if block.end <= from && !(block.start == block.end && !started) {
                    continue;
                }
                crate::cancel::check_current_cancel("table block shaping")?;
                let p = &self.request.paragraphs[block.paragraph];
                let local = from.saturating_sub(block.start);
                if local > p.text.len() || !p.text.is_char_boundary(local) {
                    return Err(fail("table block cursor is not a text boundary"));
                }
                let (shaped, metrics, steps, _) = self.cell_lines(block.paragraph, local, width)?;
                if shaped.is_empty() && p.text.is_empty() {
                    lines.push(FlowLine {
                        paragraph: block.paragraph,
                        bytes: 0..0,
                        cell_start: block.start,
                        cell_end: block.end,
                        metric: LineMetrics {
                            advance: 0.0,
                            left_pad: 0.0,
                            right_pad: 0.0,
                            ascent: 0.0,
                            descent: 0.0,
                        },
                        before: p.space_before,
                        after: p.space_after,
                        advance: p.line_height,
                        empty: true,
                    });
                } else {
                    for (i, line) in shaped.iter().enumerate() {
                        if lines.len() >= 200_000 {
                            return Err(fail("table cell block line budget exceeded"));
                        }
                        let last = line.bytes.end == p.text.len();
                        lines.push(FlowLine {
                            paragraph: block.paragraph,
                            bytes: line.bytes.clone(),
                            cell_start: block.start + line.bytes.start,
                            cell_end: if last {
                                block.end
                            } else {
                                block.start + line.bytes.end
                            },
                            metric: metrics[i],
                            before: if line.bytes.start == 0 {
                                p.space_before
                            } else {
                                0.0
                            },
                            after: if last { p.space_after } else { 0.0 },
                            advance: steps[i],
                            empty: false,
                        });
                    }
                }
            }
            if lines.len() > 200_000
                || lines.last().map_or(from, |l| l.cell_end) != self.cell_lengths[cell]
            {
                return Err(fail("incomplete/budgeted table block flow"));
            }
            let mut prefix = Vec::with_capacity(lines.len() + 1);
            prefix.push(0.0);
            for line in &lines {
                prefix.push(prefix.last().unwrap() + line.height());
            }
            let entry = CachedFlow {
                lines: lines.into(),
                prefix: prefix.into(),
            };
            let mut flows = self.flows.borrow_mut();
            let old = flows.remove(&key).map_or(0, |v| v.lines.len());
            let retained = self.cached_flow_lines.get().saturating_sub(old);
            if retained.saturating_add(entry.lines.len()) > 200_000 || flows.len() >= 8192 {
                flows.clear();
                self.cached_flow_lines.set(0);
            } else {
                self.cached_flow_lines.set(retained);
            }
            self.cached_flow_lines
                .set(self.cached_flow_lines.get() + entry.lines.len());
            flows.insert(key, entry.clone());
            (entry, 0)
        };
        let height = cache.prefix.last().unwrap() - cache.prefix[start];
        Ok((
            LineSlice {
                data: cache.lines,
                start,
            },
            height,
        ))
    }

    /// Backtrack only to a boundary satisfying the current paragraph's keep
    /// and line-count constraints. Different-width continuation is measured in
    /// the original paragraph bidi context, not as a new standalone string.
    pub(super) fn safe_block_take(
        &self,
        c: &CellLines,
        mut take: usize,
        width: f64,
    ) -> Result<usize> {
        while take > 0 && take < c.lines.len() {
            crate::cancel::check_current_cancel("table paragraph break constraints")?;
            let prev = &c.lines[take - 1];
            let next = &c.lines[take];
            let p = &self.request.paragraphs[prev.paragraph];
            let start = c.lines[..take].partition_point(|line| line.paragraph != prev.paragraph);
            // Paragraph IDs need not be numerically ordered; partition_point's
            // predicate is true only on the prefix before this contiguous block.
            if prev.paragraph != next.paragraph {
                if p.keep_with_next {
                    take = start;
                    continue;
                }
                break;
            }
            if p.keep_together {
                take = start;
                continue;
            }
            let count = take - start;
            if count < p.orphans {
                take = start;
                continue;
            }
            let remaining = break_story_lines(
                &self.paragraphs[prev.paragraph],
                &self.spans[prev.paragraph],
                self.fonts,
                &self.font_metrics,
                p,
                self.request.writing_mode,
                prev.bytes.end,
                width,
                p.widows.max(1),
            )?;
            if remaining.len() >= p.widows {
                break;
            }
            take -= 1;
        }
        Ok(take)
    }
}
