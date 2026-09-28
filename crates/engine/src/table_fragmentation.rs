//! A table fragment is a band of rows, not a clipped rendering of a tall group.
//! Spanning cells carry their logical byte cursor across bands. Row boundaries
//! and permitted within-row breaks constrain geometry; glyphs break only at
//! shaped line boundaries. Remaining content is measured again at the next
//! frame's actual width, reusing paragraph bidi and break-opportunity indexes.
use super::*;

pub(super) struct Fragment {
    pub height: f64,
    pub takes: Vec<usize>,
    pub next_row: usize,
    pub row_height_used: f64,
}

impl Prepared<'_> {
    pub(super) fn fragment(
        &self,
        group: &Group,
        available: f64,
        next_columns: &[f64],
        previous_row_height: f64,
    ) -> Result<Option<Fragment>> {
        if available <= 1e-7
            || group
                .range
                .clone()
                .any(|row| self.table.rows[row].keep_with_next)
        {
            return Ok(None);
        }
        // A non-splittable row may move to the next frame as a whole even when
        // a cell started in an earlier row. A rowspan is not itself a keep rule.
        let mut height = available.min(*group.prefix.last().unwrap());
        let containing = (0..group.range.len()).find(|&row| {
            group.prefix[row] < height - 1e-7 && group.prefix[row + 1] > height + 1e-7
        });
        if let Some(row) = containing {
            if !self.table.rows[group.range.start + row].allow_split {
                height = group.prefix[row];
            }
        }
        if height <= 1e-7 {
            return Ok(None);
        }
        let mut takes = Vec::with_capacity(group.cells.len());
        let mut text_progress = false;
        for c in &group.cells {
            crate::cancel::check_current_cancel("table rowspan fragmentation")?;
            let cell = &self.table.cells[c.cell];
            let start = group.prefix[cell.row.saturating_sub(group.range.start)];
            let end = group.prefix[cell.row + cell.row_span - group.range.start];
            let extent = height.min(end) - start;
            if extent <= 1e-7 {
                takes.push(0);
                continue;
            }
            let overhead = cell.padding[1] + cell.padding[3];
            let mut used = overhead;
            let mut take = 0;
            while take < c.lines.len() && used + c.lines[take].height() <= extent + 1e-7 {
                used += c.lines[take].height();
                take += 1;
            }
            if end <= height + 1e-7 {
                // The boundary graph reserves the full height of ending cells.
                // Never advance beyond a row while any of its cells loses text.
                if take != c.lines.len() {
                    return Err(fail("completed table cell does not fit its row band"));
                }
            } else {
                let width = next_columns[cell.column + cell.column_span]
                    - next_columns[cell.column]
                    - cell.padding[0]
                    - cell.padding[2];
                if width <= 0.0 {
                    return Err(fail("next table frame leaves no cell text width"));
                }
                take = self.safe_block_take(c, take, width)?;
                // Zero text is legitimate in a continued rowspan: other rows
                // can advance beside it. It is not by itself forward progress.
            }
            text_progress |= take > 0;
            takes.push(take);
        }
        let finished = group
            .prefix
            .iter()
            .enumerate()
            .skip(1)
            .take_while(|(_, y)| **y <= height + 1e-7)
            .last()
            .map_or(0, |(row, _)| row);
        let next_row = group.range.start + finished;
        let used = if finished == 0 {
            previous_row_height
        } else {
            0.0
        } + height
            - group.prefix[finished];
        let minimum_progress = finished == 0
            && previous_row_height < self.table.rows[group.range.start].min_height
            && used > previous_row_height + 1e-7;
        if !text_progress && finished == 0 && !minimum_progress {
            return Ok(None);
        }
        Ok(Some(Fragment {
            height,
            takes,
            next_row,
            row_height_used: used,
        }))
    }
}
