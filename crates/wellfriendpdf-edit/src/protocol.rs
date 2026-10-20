use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::PathBuf;
use crate::{
    CreateFormFieldRequest, CropImageRequest, DeleteFormFieldRequest, EditAnnotationRequest,
    EditBookmarkRequest, EditFormFieldGeometryRequest, EditFormFieldLabelRequest,
    EditFormFieldOptionsRequest, EditFormFieldValueRequest, EditLinkRequest, EditMetadataRequest,
    EditPageGeometryRequest, EditVectorPathRequest, MoveImageRequest, MutatePagesRequest,
    PageMutation, PdfEditError, ReflowTextRequest, ReorderPagesRequest,
    RepairFormFieldAppearanceRequest, ReplaceImageRequest, ReplaceTextRequest,
    SetImageOpacityRequest, TransformImageRequest, create_form_field, crop_image,
    delete_form_field, edit_annotation, edit_bookmark, edit_form_field_geometry,
    edit_form_field_label, edit_form_field_options, edit_form_field_value, edit_link,
    edit_metadata, edit_page_geometry, edit_vector_path, inspect_pdf_for_page, move_image,
    mutate_pages, reflow_text, reorder_pages, repair_form_field_appearance, replace_image,
    replace_text, set_image_opacity, transform_image,
};

const MAX_COMMAND_BYTES: u64 = 256 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "SCREAMING_SNAKE_CASE")]
enum Command {
    Health,
    Inspect {
        input_path: PathBuf,
        #[serde(default)]
        text_page: Option<u32>,
    },
    ReplaceText {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::TextRunSelector,
        replacement_text: String,
        #[serde(default)]
        text_style: Option<crate::styled_text::TextStyle>,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    ReflowText {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::TextRunSelector,
        replacement_text: String,
        region: wellfriend_pdf_edit::PdfBox,
        alignment: wellfriend_pdf_edit::PdfTextAlignment,
        line_spacing: f64,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    ReorderPages {
        input_path: PathBuf,
        output_path: PathBuf,
        page_order: Vec<u32>,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditPageGeometry {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::PageGeometrySelector,
        media_box: wellfriend_pdf_edit::PdfBox,
        crop_box: wellfriend_pdf_edit::PdfBox,
        rotation: i32,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    MutatePages {
        input_path: PathBuf,
        output_path: PathBuf,
        mutation: PageMutation,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    ReplaceImage {
        input_path: PathBuf,
        output_path: PathBuf,
        replacement_image_path: PathBuf,
        selector: wellfriend_pdf_edit::ImageSelector,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    MoveImage {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::ImagePlacementSelector,
        destination_x: f64,
        destination_y: f64,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    TransformImage {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::ImagePlacementSelector,
        destination_matrix: wellfriend_pdf_edit::PdfMatrix,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    CropImage {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::ImagePlacementSelector,
        destination_crop: wellfriend_pdf_edit::PdfNormalizedCrop,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    SetImageOpacity {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::ImagePlacementSelector,
        destination_opacity: f64,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditVectorPath {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::VectorPathSelector,
        mutation: wellfriend_pdf_edit::VectorPathMutation,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditLink {
        input_path: PathBuf,
        output_path: PathBuf,
        mutation: wellfriend_pdf_edit::LinkMutation,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditAnnotation {
        input_path: PathBuf,
        output_path: PathBuf,
        mutation: wellfriend_pdf_edit::AnnotationMutation,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditMetadata {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::PdfMetadataSelector,
        values: wellfriend_pdf_edit::PdfMetadataSpec,
        xmp_mode: wellfriend_pdf_edit::PdfXmpMode,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditBookmark {
        input_path: PathBuf,
        output_path: PathBuf,
        mutation: wellfriend_pdf_edit::BookmarkMutation,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditFormFieldValue {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormFieldSelector,
        value: Option<String>,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditFormFieldGeometry {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormFieldSelector,
        widget: wellfriend_pdf_edit::FormWidgetSelector,
        rectangle: wellfriend_pdf_edit::PdfBox,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditFormFieldLabel {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormFieldSelector,
        label: Option<String>,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    EditFormFieldOptions {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormFieldSelector,
        options: Vec<String>,
        value: String,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    RepairFormFieldAppearance {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormFieldSelector,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    CreateFormField {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormCreationSelector,
        name: String,
        label: Option<String>,
        kind: wellfriend_pdf_edit::PdfFormFieldKind,
        value: Option<String>,
        options: Vec<String>,
        widgets: Vec<wellfriend_pdf_edit::CreateFormFieldWidget>,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
    DeleteFormField {
        input_path: PathBuf,
        output_path: PathBuf,
        selector: wellfriend_pdf_edit::FormFieldSelector,
        #[serde(default)]
        allow_signature_invalidation: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Success<T> {
    status: &'static str,
    result: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Failure<'a> {
    status: &'static str,
    code: &'a str,
    message: String,
    retryable: bool,
}

/// Runs one editor command read as JSON from stdin and prints one JSON
/// envelope on stdout. Returns the process exit code: 0 on success, 2 on a
/// refused or failed edit.
pub fn run_stdin() -> i32 {
    if let Err(error) = run() {
        let failure = Failure {
            status: "ERROR",
            code: error.code(),
            message: error.to_string(),
            retryable: false,
        };
        println!(
            "{}",
            serde_json::to_string(&failure).expect("failure envelope must serialize")
        );
        return 2;
    }
    0
}

fn run() -> Result<(), PdfEditError> {
    let mut payload = Vec::new();
    io::stdin()
        .take(MAX_COMMAND_BYTES + 1)
        .read_to_end(&mut payload)
        .map_err(|_| PdfEditError::InputUnavailable)?;
    if payload.len() as u64 > MAX_COMMAND_BYTES {
        return Err(PdfEditError::InputTooLarge);
    }
    let command: Command =
        serde_json::from_slice(&payload).map_err(|_| PdfEditError::InvalidPdf)?;
    match command {
        Command::Health => {
            println!(
                "{{\"status\":\"SUCCESS\",\"result\":{{\"version\":\"{}\"}}}}",
                env!("CARGO_PKG_VERSION")
            );
        }
        Command::Inspect {
            input_path,
            text_page,
        } => {
            let result = inspect_pdf_for_page(&input_path, text_page)?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::ReplaceText {
            input_path,
            output_path,
            selector,
            replacement_text,
            text_style,
            allow_signature_invalidation,
        } => {
            let request = ReplaceTextRequest {
                input_path,
                output_path,
                selector,
                replacement_text,
                allow_signature_invalidation,
            };
            let result = if let Some(style) = text_style {
                crate::styled_text::replace_styled_text(&request, &style)?
            } else {
                match replace_text(&request) {
                    Ok(result) => serde_json::to_value(result)
                        .map_err(|_| PdfEditError::ValidationFailed)?,
                    // The run's embedded font cannot draw the replacement:
                    // write it in the full font of the same family and size.
                    Err(PdfEditError::GlyphUnavailable) => {
                        crate::full_font::replace_text_full_font(&request)?
                    }
                    Err(error) => return Err(error),
                }
            };
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::ReflowText {
            input_path,
            output_path,
            selector,
            replacement_text,
            region,
            alignment,
            line_spacing,
            allow_signature_invalidation,
        } => {
            let result = reflow_text(&ReflowTextRequest {
                input_path,
                output_path,
                selector,
                replacement_text,
                region,
                alignment,
                line_spacing,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::ReorderPages {
            input_path,
            output_path,
            page_order,
            allow_signature_invalidation,
        } => {
            let result = reorder_pages(&ReorderPagesRequest {
                input_path,
                output_path,
                page_order,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditPageGeometry {
            input_path,
            output_path,
            selector,
            media_box,
            crop_box,
            rotation,
            allow_signature_invalidation,
        } => {
            let result = edit_page_geometry(&EditPageGeometryRequest {
                input_path,
                output_path,
                selector,
                media_box,
                crop_box,
                rotation,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::MutatePages {
            input_path,
            output_path,
            mutation,
            allow_signature_invalidation,
        } => {
            let result = mutate_pages(&MutatePagesRequest {
                input_path,
                output_path,
                mutation,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::ReplaceImage {
            input_path,
            output_path,
            replacement_image_path,
            selector,
            allow_signature_invalidation,
        } => {
            let result = replace_image(&ReplaceImageRequest {
                input_path,
                output_path,
                replacement_image_path,
                selector,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::MoveImage {
            input_path,
            output_path,
            selector,
            destination_x,
            destination_y,
            allow_signature_invalidation,
        } => {
            let result = move_image(&MoveImageRequest {
                input_path,
                output_path,
                selector,
                destination_x,
                destination_y,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::TransformImage {
            input_path,
            output_path,
            selector,
            destination_matrix,
            allow_signature_invalidation,
        } => {
            let result = transform_image(&TransformImageRequest {
                input_path,
                output_path,
                selector,
                destination_matrix,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::CropImage {
            input_path,
            output_path,
            selector,
            destination_crop,
            allow_signature_invalidation,
        } => {
            let result = crop_image(&CropImageRequest {
                input_path,
                output_path,
                selector,
                destination_crop,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::SetImageOpacity {
            input_path,
            output_path,
            selector,
            destination_opacity,
            allow_signature_invalidation,
        } => {
            let result = set_image_opacity(&SetImageOpacityRequest {
                input_path,
                output_path,
                selector,
                destination_opacity,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditVectorPath {
            input_path,
            output_path,
            selector,
            mutation,
            allow_signature_invalidation,
        } => {
            let result = edit_vector_path(&EditVectorPathRequest {
                input_path,
                output_path,
                selector,
                mutation,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditLink {
            input_path,
            output_path,
            mutation,
            allow_signature_invalidation,
        } => {
            let result = edit_link(&EditLinkRequest {
                input_path,
                output_path,
                mutation,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditAnnotation {
            input_path,
            output_path,
            mutation,
            allow_signature_invalidation,
        } => {
            let result = edit_annotation(&EditAnnotationRequest {
                input_path,
                output_path,
                mutation,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditMetadata {
            input_path,
            output_path,
            selector,
            values,
            xmp_mode,
            allow_signature_invalidation,
        } => {
            let result = edit_metadata(&EditMetadataRequest {
                input_path,
                output_path,
                selector,
                values,
                xmp_mode,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditBookmark {
            input_path,
            output_path,
            mutation,
            allow_signature_invalidation,
        } => {
            let result = edit_bookmark(&EditBookmarkRequest {
                input_path,
                output_path,
                mutation,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditFormFieldValue {
            input_path,
            output_path,
            selector,
            value,
            allow_signature_invalidation,
        } => {
            let result = edit_form_field_value(&EditFormFieldValueRequest {
                input_path,
                output_path,
                selector,
                value,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditFormFieldGeometry {
            input_path,
            output_path,
            selector,
            widget,
            rectangle,
            allow_signature_invalidation,
        } => {
            let result = edit_form_field_geometry(&EditFormFieldGeometryRequest {
                input_path,
                output_path,
                selector,
                widget,
                rectangle,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditFormFieldLabel {
            input_path,
            output_path,
            selector,
            label,
            allow_signature_invalidation,
        } => {
            let result = edit_form_field_label(&EditFormFieldLabelRequest {
                input_path,
                output_path,
                selector,
                label,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::EditFormFieldOptions {
            input_path,
            output_path,
            selector,
            options,
            value,
            allow_signature_invalidation,
        } => {
            let result = edit_form_field_options(&EditFormFieldOptionsRequest {
                input_path,
                output_path,
                selector,
                options,
                value,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::RepairFormFieldAppearance {
            input_path,
            output_path,
            selector,
            allow_signature_invalidation,
        } => {
            let result = repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
                input_path,
                output_path,
                selector,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::CreateFormField {
            input_path,
            output_path,
            selector,
            name,
            label,
            kind,
            value,
            options,
            widgets,
            required,
            allow_signature_invalidation,
        } => {
            let result = create_form_field(&CreateFormFieldRequest {
                input_path,
                output_path,
                selector,
                name,
                label,
                kind,
                value,
                options,
                widgets,
                required,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
        Command::DeleteFormField {
            input_path,
            output_path,
            selector,
            allow_signature_invalidation,
        } => {
            let result = delete_form_field(&DeleteFormFieldRequest {
                input_path,
                output_path,
                selector,
                allow_signature_invalidation,
            })?;
            println!(
                "{}",
                serde_json::to_string(&Success {
                    status: "SUCCESS",
                    result
                })
                .map_err(|_| PdfEditError::ValidationFailed)?
            );
        }
    }
    Ok(())
}
