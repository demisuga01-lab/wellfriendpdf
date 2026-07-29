# Transparency Closeout Transparency Closure Audit

Transparency Closeout closes the remaining Transparency Rendering-owned transparency items. Evidence is
machine-readable in
`target/transparency_rendering-transparency-compositing/transparency_closeout-closure-audit.json`.

| Area | Previous status | Target status | Implementation result | Tests/artifacts | Remaining limit |
|---|---|---|---|---|---|
| `alpha_image` | Wellfriend was an outlier while references agreed | Closed | Image XObject painting now multiplies decoded/SMask alpha by graphics-state `/ca` before `PixelBuffer::blend_pixel` | `alpha_image`; `transparency_closeout-render-results.json` | None for DeviceRGB alpha constants |
| `soft_mask_matte_background` | Matte/background edge cases were partial | Closed or unsupported-reported | Image `/SMask /Matte` unblends common DeviceGray/RGB/CMYK matte values; ExtGState `/BC` remains implemented | `image_smask_matte`, `softmask_alpha_bc_background`; `transparency_closeout-transparency-matrix.json` | Advanced ICC/device-link matte conversion is CMM work |
| `luminosity_soft_mask_color_space` | Exact color-managed luminosity was partial | Closed for common spaces | DeviceGray, DeviceRGB, and DeviceCMYK mask groups paint through the current converter before Rec.601 luminosity extraction | `softmask_luminosity_devicegray`, `softmask_luminosity_devicergb`, `softmask_luminosity_devicecmyk` | ICC/calibrated exact CMM parity |
| `transparency_group_color_space` | Mostly device-space wording | Closed for common device spaces | Explicit DeviceGray/RGB/CMYK group `/CS` is recognized and exercised through the group stack for common source colors | `group_colorspace_devicegray`, `group_colorspace_devicergb`, `group_colorspace_devicecmyk` | Full ICC/device-link/multicolor group blending |
| `interior_knockout_overlap` | Exact interior overlap partial | Closed for supported vector/Form groups | Knockout group buffers retain an initial backdrop and each covered pixel recomposes against it | `knockout_overlap_exact`, `knockout_overlap_nested_form`; buffer unit test | Text clipping and pattern/shading paints inside knockout groups |
| `multi_reference_closure` | One `alpha_image` outlier plus partial rows | Closed | Poppler/PDFium/MuPDF/Wellfriend audit rerun on Transparency Rendering plus 07B fixtures | `transparency_closeout-reference-disagreement-summary.json`, `transparency_closeout-html-report/index.html` | Malformed recursive group remains classified as malformed/reference failure |

Transparency Closeout audit result: 47 fixtures, 41
`all_references_agree_and_wellfriendpdf_passes`, 5
`references_disagree_and_wellfriendpdf_within_cluster`, 1
`malformed_or_reference_failure`, 0 Wellfriend-outlier failures, and 0
unclassified failures.
