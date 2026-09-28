//! Common shading dictionary entries and a constant-storage temporary BBox clip.
use super::*;
use std::borrow::Cow;

pub(super) struct CommonEntries {
    pub(super) bbox: Option<[f64; 4]>,
    pub(super) background: Option<Vec<f64>>,
}

fn resolved<'a>(object: &'a PdfObject, reader: &PdfReader) -> Result<Cow<'a, PdfObject>, String> {
    match object {
        PdfObject::Reference { .. } => reader
            .resolve(object.clone())
            .map(Cow::Owned)
            .map_err(|error| error.to_string()),
        _ => Ok(Cow::Borrowed(object)),
    }
}
fn optional_numbers(
    dict: &PdfDictionary,
    key: &str,
    reader: &PdfReader,
    max: usize,
) -> Result<Option<Vec<f64>>, String> {
    let Some(object) = dict.get(key) else {
        return Ok(None);
    };
    let object = resolved(object, reader)?;
    if matches!(object.as_ref(), PdfObject::Null) {
        return Ok(None);
    }
    let array = object
        .as_array()
        .ok_or_else(|| format!("shading /{key} must be a numeric array"))?;
    if array.len() > max {
        return Err(format!("shading /{key} has too many components"));
    }
    let mut numbers = Vec::with_capacity(array.len());
    for value in array {
        crate::cancel::check_current_cancel("shading common dictionary entries")
            .map_err(|error| error.to_string())?;
        let value = resolved(value, reader)?;
        let value = value
            .as_number()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("shading /{key} contains a nonfinite or nonnumeric value"))?;
        numbers.push(value);
    }
    Ok(Some(numbers))
}
impl CommonEntries {
    pub(super) fn read(dict: &PdfDictionary, reader: &PdfReader) -> Result<Self, String> {
        let bbox = optional_numbers(dict, "BBox", reader, 4)?
            .map(|values| {
                if values.len() != 4 {
                    return Err("shading /BBox requires four coordinates".to_string());
                }
                Ok([
                    values[0].min(values[2]),
                    values[1].min(values[3]),
                    values[0].max(values[2]),
                    values[1].max(values[3]),
                ])
            })
            .transpose()?;
        let background = optional_numbers(
            dict,
            "Background",
            reader,
            crate::render::colorspace::MAX_DEVICEN_COMPONENTS,
        )?;
        if let Some(object) = dict.get("AntiAlias") {
            let value = resolved(object, reader)?;
            if !matches!(value.as_ref(), PdfObject::Null | PdfObject::Boolean(_)) {
                return Err("shading /AntiAlias must be boolean".into());
            }
        }
        Ok(Self { bbox, background })
    }
}

#[cfg(test)]
#[path = "shading_region_tests.rs"]
mod tests;

type Point = (f64, f64);
type Bounds = (i32, i32, i32, i32);
const MAX_VERTICES: usize = 16;

#[derive(Clone, Copy, Debug)]
struct Polygon {
    points: [Point; MAX_VERTICES],
    len: usize,
}
impl Polygon {
    fn empty() -> Self {
        Self {
            points: [(0.0, 0.0); MAX_VERTICES],
            len: 0,
        }
    }
    fn push(&mut self, point: Point) -> Result<(), &'static str> {
        if !point.0.is_finite() || !point.1.is_finite() {
            return Err("nonfinite shading clip geometry");
        }
        if self.len > 0 && self.points[self.len - 1] == point {
            return Ok(());
        }
        if self.len == MAX_VERTICES {
            return Err("shading clip vertex limit exceeded");
        }
        self.points[self.len] = point;
        self.len += 1;
        Ok(())
    }
    fn clip(self, axis: usize, bound: f64, keep_greater: bool) -> Result<Self, &'static str> {
        let mut output = Self::empty();
        if self.len == 0 {
            return Ok(output);
        }
        let coordinate = |point: Point| if axis == 0 { point.0 } else { point.1 };
        let inside = |point: Point| {
            if keep_greater {
                coordinate(point) >= bound
            } else {
                coordinate(point) <= bound
            }
        };
        let mut previous = self.points[self.len - 1];
        let mut previous_inside = inside(previous);
        for current in &self.points[..self.len] {
            let current_inside = inside(*current);
            if previous_inside != current_inside {
                let (a, b) = (coordinate(previous), coordinate(*current));
                // Normalizing before subtraction also handles finite endpoints
                // whose unscaled difference would overflow.
                let scale = a.abs().max(b.abs()).max(bound.abs()).max(1.0);
                let t = ((bound / scale - a / scale) / (b / scale - a / scale)).clamp(0.0, 1.0);
                if !t.is_finite() {
                    return Err("invalid shading clip intersection");
                }
                let mut point = if t == 0.0 {
                    previous
                } else if t == 1.0 {
                    *current
                } else {
                    (
                        (1.0 - t) * previous.0 + t * current.0,
                        (1.0 - t) * previous.1 + t * current.1,
                    )
                };
                if axis == 0 {
                    point.0 = bound;
                } else {
                    point.1 = bound;
                }
                output.push(point)?;
            }
            if current_inside {
                output.push(*current)?;
            }
            previous = *current;
            previous_inside = current_inside;
        }
        if output.len > 1 && output.points[0] == output.points[output.len - 1] {
            output.len -= 1;
        }
        Ok(output)
    }
    fn rectangle(self, bounds: [f64; 4]) -> Result<Self, &'static str> {
        self.clip(0, bounds[0], true)?
            .clip(0, bounds[2], false)?
            .clip(1, bounds[1], true)?
            .clip(1, bounds[3], false)
    }
    fn area(&self) -> f64 {
        if self.len < 3 {
            return 0.0;
        }
        let origin = self.points[0];
        let mut area = 0.0;
        // Translating to a local origin avoids cancellation between large
        // absolute-coordinate products, particularly for small viewport tiles.
        for i in 1..self.len - 1 {
            let a = (self.points[i].0 - origin.0, self.points[i].1 - origin.1);
            let b = (
                self.points[i + 1].0 - origin.0,
                self.points[i + 1].1 - origin.1,
            );
            area += a.0 * b.1 - a.1 * b.0;
        }
        (area * 0.5).abs()
    }
    fn bounds(&self) -> Option<[f64; 4]> {
        if self.len < 3 || self.area() == 0.0 {
            return None;
        }
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for (x, y) in &self.points[..self.len] {
            bounds[0] = bounds[0].min(*x);
            bounds[1] = bounds[1].min(*y);
            bounds[2] = bounds[2].max(*x);
            bounds[3] = bounds[3].max(*y);
        }
        Some(bounds)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PaintRegion {
    polygon: Option<Polygon>,
    axis_aligned: Option<[f64; 4]>,
    bounds: Option<Bounds>,
}
impl PaintRegion {
    pub(super) fn new(
        bbox: Option<[f64; 4]>,
        ctm: &Transform2D,
        viewport: &Viewport,
        target: Bounds,
    ) -> Result<Self, String> {
        let Some([x0, y0, x1, y1]) = bbox else {
            return Ok(Self {
                polygon: None,
                axis_aligned: None,
                bounds: Some(target),
            });
        };
        if x0 == x1 || y0 == y1 {
            return Ok(Self {
                polygon: Some(Polygon::empty()),
                axis_aligned: None,
                bounds: None,
            });
        }
        // /BBox belongs to target space, not Type 1's function /Matrix domain.
        let transform = ctm.concat(&viewport.to_transform());
        let mut polygon = Polygon::empty();
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            polygon.push(transform.transform_point(x, y))?;
        }
        let [a, b, c, d, _, _] = transform.to_array();
        let aligned = (b == 0.0 && c == 0.0) || (a == 0.0 && d == 0.0);
        let polygon = polygon.rectangle([
            target.0 as f64,
            target.1 as f64,
            target.2 as f64,
            target.3 as f64,
        ])?;
        let continuous = polygon.bounds();
        let bounds = continuous
            .map(|b| {
                (
                    (b[0].floor() as i32).max(target.0),
                    (b[1].floor() as i32).max(target.1),
                    (b[2].ceil() as i32).min(target.2),
                    (b[3].ceil() as i32).min(target.3),
                )
            })
            .filter(|(x0, y0, x1, y1)| x1 > x0 && y1 > y0);
        Ok(Self {
            polygon: Some(polygon),
            axis_aligned: if aligned { continuous } else { None },
            bounds,
        })
    }
    pub(super) fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }
    pub(super) fn has_bbox(&self) -> bool {
        self.polygon.is_some()
    }
    pub(super) fn shifted(mut self, x: i32, y: i32) -> Self {
        if let Some(polygon) = &mut self.polygon {
            for p in &mut polygon.points[..polygon.len] {
                p.0 -= x as f64;
                p.1 -= y as f64;
            }
        }
        if let Some(b) = &mut self.axis_aligned {
            b[0] -= x as f64;
            b[2] -= x as f64;
            b[1] -= y as f64;
            b[3] -= y as f64;
        }
        self.bounds = self
            .bounds
            .map(|(x0, y0, x1, y1)| (x0 - x, y0 - y, x1 - x, y1 - y));
        self
    }
    pub(super) fn coverage(&self, x: i32, y: i32) -> Result<f32, &'static str> {
        let Some(polygon) = self.polygon else {
            return Ok(1.0);
        };
        let (x, y) = (x as f64, y as f64);
        if let Some(b) = self.axis_aligned {
            return Ok(((b[2].min(x + 1.0) - b[0].max(x)).max(0.0)
                * (b[3].min(y + 1.0) - b[1].max(y)).max(0.0))
            .clamp(0.0, 1.0) as f32);
        }
        let area = polygon.rectangle([x, y, x + 1.0, y + 1.0])?.area();
        if !area.is_finite() {
            return Err("shading BBox coverage overflow");
        }
        Ok(area.clamp(0.0, 1.0) as f32)
    }
}
