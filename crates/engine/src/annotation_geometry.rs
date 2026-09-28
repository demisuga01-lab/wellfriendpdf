//! Source-space geometry shared by story translation and standalone resize.
//! Appearance Form streams are deliberately unchanged: PDF appearance mapping
//! fits their Matrix-transformed BBox to the new annotation Rect.
use super::*;

pub(super) fn affine(old: [f64; 4], new: [f64; 4]) -> Result<[f64; 4]> {
    if old.iter().chain(new.iter()).any(|v| !v.is_finite())
        || old[0] >= old[2]
        || old[1] >= old[3]
        || new[0] >= new[2]
        || new[1] >= new[3]
    {
        return Err(fail(
            "annotation transform requires normalized finite nonempty rectangles",
        ));
    }
    let sx = (new[2] - new[0]) / (old[2] - old[0]);
    let sy = (new[3] - new[1]) / (old[3] - old[1]);
    let result = [sx, sy, new[0] - sx * old[0], new[1] - sy * old[1]];
    if result.iter().any(|v| !v.is_finite()) || sx <= 0.0 || sy <= 0.0 {
        return Err(fail("annotation transform overflow or underflow"));
    }
    Ok(result)
}

pub(super) fn transform(
    reader: &crate::reader::PdfReader,
    entry: &Entry,
    new: [f64; 4],
) -> Result<PdfDictionary> {
    let [sx, sy, tx, ty] = affine(entry.source.rect, new)?;
    let resized = (sx - 1.0).abs().max((sy - 1.0).abs()) > 1e-9;
    // Scalar leader lengths do not express an arbitrary anisotropic transform.
    // Translation and uniform resize keep their original semantic direction.
    let uniform = (sx - sy).abs() <= 1e-9 * sx.abs().max(sy.abs()).max(1.0);
    if resized
        && !uniform
        && ["LL", "LLE", "LLO", "CO"]
            .iter()
            .any(|k| entry.dict.contains_key(k))
    {
        return Err(fail(
            "line leader and caption geometry requires uniform resize",
        ));
    }
    let mut dict = entry.dict.clone();
    let map = |values: &mut [f64]| -> Result<()> {
        for point in values.chunks_exact_mut(2) {
            point[0] = point[0] * sx + tx;
            point[1] = point[1] * sy + ty;
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(fail("annotation coordinate overflow"));
        }
        Ok(())
    };
    for key in ["QuadPoints", "Vertices", "L", "CL"] {
        if let Some(v) = dict.get(key) {
            let mut n = finite_array(reader, v)?;
            let valid = match key {
                "L" => n.len() == 4,
                "QuadPoints" => !n.is_empty() && n.len() % 8 == 0,
                "CL" => n.len() == 4 || n.len() == 6,
                "Vertices" => {
                    n.len()
                        >= (if entry.source.subtype == "Polygon" {
                            6
                        } else {
                            4
                        })
                        && n.len() % 2 == 0
                }
                _ => false,
            };
            if !valid {
                return Err(fail("invalid annotation coordinate count"));
            }
            map(&mut n)?;
            dict.insert(key, array(n));
        }
    }
    if let Some(v) = dict.get("InkList") {
        let object = reader.resolve(v.clone())?;
        let strokes = object.as_array().ok_or_else(|| fail("invalid InkList"))?;
        if strokes.len() > 100_000 {
            return Err(fail("ink stroke budget exceeded"));
        }
        let mut count = 0usize;
        let mut transformed = Vec::new();
        for stroke in strokes {
            crate::cancel::check_current_cancel("annotation ink geometry")?;
            let mut n = finite_array(reader, stroke)?;
            count = count.saturating_add(n.len());
            if count > 200_000 || n.len() % 2 != 0 {
                return Err(fail("invalid ink coordinate count or budget exceeded"));
            }
            map(&mut n)?;
            transformed.push(array(n));
        }
        dict.insert("InkList", PdfObject::Array(transformed));
    }
    if resized {
        if let Some(v) = dict.get("RD") {
            let mut n = finite_array(reader, v)?;
            let old = entry.source.rect;
            if n.len() != 4
                || n.iter().any(|v| *v < 0.0)
                || n[0] + n[2] >= old[2] - old[0]
                || n[1] + n[3] >= old[3] - old[1]
            {
                return Err(fail("invalid annotation rectangle differences"));
            }
            // RD is a vector of edge distances, not two absolute points.
            for (i, v) in n.iter_mut().enumerate() {
                *v *= if i % 2 == 0 { sx } else { sy };
            }
            if n.iter().any(|v| !v.is_finite()) {
                return Err(fail("annotation inset overflow"));
            }
            dict.insert("RD", array(n));
        }
        for key in ["LL", "LLE", "LLO"] {
            if let Some(v) = dict.get(key) {
                let object = reader.resolve(v.clone())?;
                let value = object
                    .as_number()
                    .ok_or_else(|| fail("invalid annotation leader length"))?
                    * sx;
                if !value.is_finite() || key != "LL" && value < 0.0 {
                    return Err(fail("invalid annotation leader length"));
                }
                dict.insert(key, PdfObject::Real(value));
            }
        }
        if let Some(v) = dict.get("CO") {
            let mut n = finite_array(reader, v)?;
            if n.len() != 2 {
                return Err(fail("invalid annotation caption offset"));
            }
            for v in &mut n {
                *v *= sx;
            }
            if n.iter().any(|v| !v.is_finite()) {
                return Err(fail("annotation caption offset overflow"));
            }
            dict.insert("CO", array(n));
        }
    }
    dict.insert("Rect", array(new));
    Ok(dict)
}
