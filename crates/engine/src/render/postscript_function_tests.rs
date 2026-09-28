//! Source assertions, not executed qualification.
use super::*;
use crate::render::vector_fallback::{
    VectorPostScriptCmykStitchingSegment, VectorPostScriptStitchingSegment,
    VectorPostScriptTintStitchingSegment,
};

#[test]
fn exact_stitching_writers_emit_parent_ranges_for_all_channel_models() {
    let rgb = VectorPostScriptStitchingFunction {
        domain: [0.0, 1.0],
        range: Some([[0.25, 0.75]; 3]),
        segments: vec![VectorPostScriptStitchingSegment {
            bound_end: 1.0,
            encode: [0.0, 1.0],
            function: VectorPostScriptType2Function {
                domain: [0.0, 1.0],
                c0: [0.0; 3],
                c1: [1.0; 3],
                n: 1.0,
                range: None,
            },
        }],
    };
    let ps = ps_rgb_exact_stitching_function(&rgb);
    assert!(
        ps.ends_with(" /Range [0.250000 0.750000 0.250000 0.750000 0.250000 0.750000] >>"),
        "{ps}"
    );
    let cmyk = VectorPostScriptCmykStitchingFunction {
        domain: [0.0, 1.0],
        range: Some([[0.25, 0.75]; 4]),
        segments: vec![VectorPostScriptCmykStitchingSegment {
            bound_end: 1.0,
            encode: [0.0, 1.0],
            function: VectorPostScriptType2CmykFunction {
                domain: [0.0, 1.0],
                c0: [0.0; 4],
                c1: [1.0; 4],
                n: 1.0,
                range: None,
            },
        }],
    };
    assert!(ps_cmyk_exact_stitching_function(&cmyk).ends_with(
        " /Range [0.250000 0.750000 0.250000 0.750000 0.250000 0.750000 0.250000 0.750000] >>"
    ));
    let tint = VectorPostScriptTintStitchingFunction {
        domain: [0.0, 1.0],
        range: Some([0.25, 0.75]),
        segments: vec![VectorPostScriptTintStitchingSegment {
            bound_end: 1.0,
            encode: [0.0, 1.0],
            function: VectorPostScriptType2ComponentFunction {
                domain: [0.0, 1.0],
                c0: 0.0,
                c1: 1.0,
                n: 1.0,
                range: None,
            },
        }],
    };
    assert!(ps_tint_exact_stitching_function(&tint).ends_with(" /Range [0.250000 0.750000] >>"));
}

#[test]
fn numeric_emission_does_not_collapse_close_bounds_or_nearly_linear_exponents() {
    let colour = [0.12345679_f32, 0.5, 0.50001];
    let components: Vec<f32> = ps_rgb_components(colour)
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    assert_eq!(components, colour.to_vec());
    for number in [
        1e-12,
        2e-12,
        0.5 + 1e-12,
        1.0 + 1e-12,
        -1e100,
        f64::MIN_POSITIVE,
    ] {
        for digits in [4, 6] {
            let emitted = ps_function_number(number, digits);
            assert_eq!(emitted.parse::<f64>().unwrap(), number);
        }
    }
    assert_ne!(
        ps_function_number(0.5, 6),
        ps_function_number(0.5 + 1e-12, 6)
    );
    let exact = VectorPostScriptShadingFunction::Type2(VectorPostScriptType2Function {
        domain: [0.0, 1.0],
        c0: [0.0; 3],
        c1: [1.0; 3],
        n: 1.0 + 1e-12,
        range: None,
    });
    assert!(!ps_shading_domain_clause([1e-12, 1.0], Some(&exact)).is_empty());
    assert!(ps_exact_shading_function(&exact)
        .contains(&format!("/N {}", ps_function_number(1.0 + 1e-12, 6))));
    let stops = [
        VectorShadingStop {
            offset: 0.0,
            rgb: [0.0; 3],
        },
        VectorShadingStop {
            offset: 1e-12,
            rgb: [0.25; 3],
        },
        VectorShadingStop {
            offset: 2e-12,
            rgb: [0.75; 3],
        },
        VectorShadingStop {
            offset: 1.0,
            rgb: [1.0; 3],
        },
    ];
    assert!(ps_rgb_shading_function(None, &stops).contains("/Bounds [1e-12 2e-12 "));
}
