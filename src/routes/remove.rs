use axum::{
    extract::{Multipart, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use image::{DynamicImage, ImageBuffer, Luma, Rgb, RgbImage};
use ort::value::Tensor;

use crate::errors::{AppError, Result};
use crate::models::user::Claims;
use crate::state::AppState;

/// Accepted MIME types for the image field.
const ACCEPTED_MIME_TYPES: &[&str] = &["image/png", "image/jpeg", "image/jpg", "image/webp"];

/// Magic byte signatures used to validate file content independent of the
/// Content-Type header (guards against renamed/spoofed extensions).
fn detect_image_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    // WebP: "RIFF....WEBP"
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

// Helper to initialize an ONNX session with aggressive memory optimizations (fits under 512MB RAM)
fn load_onnx_session(model_path: &str) -> Result<ort::session::Session> {
    let builder = ort::session::Session::builder().map_err(|e| {
        AppError::Internal(format!("Failed to create ONNX session builder: {}", e))
    })?;
    let builder = builder
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Disable)
        .map_err(|e| {
            AppError::Internal(format!("Failed to set optimization level: {}", e))
        })?;
    let builder = builder.with_memory_pattern(false).map_err(|e| {
        AppError::Internal(format!("Failed to disable memory pattern: {}", e))
    })?;
    let builder = builder.with_config_entry("session.use_memory_arena", "0").map_err(|e| {
        AppError::Internal(format!("Failed to disable memory arena: {}", e))
    })?;
    let builder = builder.with_config_entry("session.use_arena_allocation", "0").map_err(|e| {
        AppError::Internal(format!("Failed to disable arena allocation: {}", e))
    })?;
    let mut builder = builder.with_intra_threads(1).map_err(|e| {
        AppError::Internal(format!("Failed to set intra threads: {}", e))
    })?;
    builder.commit_from_file(model_path).map_err(|e| {
        AppError::Internal(format!("Failed to load ONNX model from {}: {}", model_path, e))
    })
}

// POST /api/v1/remove-background
pub async fn remove_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<impl IntoResponse> {

    // ── 1. Optional JWT validation ────────────────────────────────────────────
    if let Some(auth_header) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
    {
        if auth_header.starts_with("Bearer ") && auth_header.len() > 7 {
            let token = auth_header[7..].trim();
            if !token.is_empty() && token != "undefined" && token != "null" {
                decode::<Claims>(
                    token,
                    &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
                    &Validation::default(),
                )
                .map_err(|e| match e.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                        AppError::Unauthorized("Your session has expired. Please sign in again.".to_string())
                    }
                    _ => AppError::Unauthorized("Invalid authorization token.".to_string()),
                })?;
            }
        }
    }

    // ── 2. Parse multipart upload ─────────────────────────────────────────────
    let mut multipart = multipart;
    let mut image_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::BadRequest(format!("Failed to parse multipart upload: {}", e))
    })? {
        let name = field.name().unwrap_or_default().to_string();

        if name == "image" {
            // Capture the declared content-type of this part (may be absent)
            let declared_content_type: Option<String> = field
                .content_type()
                .map(|ct| ct.to_string());

            // Validate declared content-type if present
            if let Some(ref ct) = declared_content_type {
                let ct_lower = ct.to_lowercase();
                let accepted = ACCEPTED_MIME_TYPES
                    .iter()
                    .any(|&m| ct_lower.starts_with(m));
                if !accepted {
                    return Err(AppError::UnsupportedMediaType(format!(
                        "Unsupported file type '{}'. Accepted formats: PNG, JPEG, WebP.",
                        ct
                    )));
                }
            }

            let data = field.bytes().await.map_err(|e| {
                AppError::BadRequest(format!("Failed to read uploaded file: {}", e))
            })?;

            // Size check: 10 MB hard limit
            const MAX_BYTES: usize = 10 * 1024 * 1024;
            if data.len() > MAX_BYTES {
                return Err(AppError::PayloadTooLarge(
                    "File size exceeds the 10 MB limit. Please upload a smaller image.".to_string(),
                ));
            }

            if data.is_empty() {
                return Err(AppError::BadRequest(
                    "Uploaded file is empty.".to_string(),
                ));
            }

            // Magic-byte validation — catches renamed files (e.g. script.png)
            if detect_image_format(&data).is_none() {
                return Err(AppError::UnsupportedMediaType(
                    "File content does not match a supported image format (PNG, JPEG, WebP). \
                     Please upload a valid image file."
                        .to_string(),
                ));
            }

            image_bytes = Some(data.to_vec());
            break;
        }
    }

    let raw_bytes = image_bytes.ok_or_else(|| {
        AppError::BadRequest(
            "No 'image' field found in the request. \
             Send the file as multipart/form-data with field name 'image'."
                .to_string(),
        )
    })?;

    // ── 3. Decode image ───────────────────────────────────────────────────────
    let original_img = image::load_from_memory(&raw_bytes).map_err(|e| {
        tracing::warn!("Image decode failed: {}", e);
        AppError::UnprocessableEntity(
            "Could not decode the uploaded image. The file may be corrupt or truncated."
                .to_string(),
        )
    })?;

    let original_width = original_img.width();
    let original_height = original_img.height();

    // Sanity-check dimensions: reject absurdly small or huge inputs
    if original_width < 4 || original_height < 4 {
        return Err(AppError::UnprocessableEntity(
            "Image dimensions are too small. Minimum size is 4×4 pixels.".to_string(),
        ));
    }
    if original_width > 8000 || original_height > 8000 {
        return Err(AppError::UnprocessableEntity(
            "Image dimensions exceed the 8000×8000 pixel limit. \
             Please downscale the image before uploading."
                .to_string(),
        ));
    }

    // ── 4. PHASE 1: u2netp — fast rough cut (320×320, ImageNet norm) ──────────
    let resized_fast = original_img.resize_exact(320, 320, image::imageops::FilterType::Triangle);
    let rgb_fast = resized_fast.to_rgb8();

    let mean_fast = [0.485f32, 0.456, 0.406];
    let std_fast  = [0.229f32, 0.224, 0.225];

    let mut tensor_fast = ndarray::Array4::<f32>::zeros((1, 3, 320, 320));
    for y in 0..320usize {
        for x in 0..320usize {
            let pixel = rgb_fast.get_pixel(x as u32, y as u32);
            for c in 0..3usize {
                tensor_fast[[0, c, y, x]] =
                    (pixel[c] as f32 / 255.0 - mean_fast[c]) / std_fast[c];
            }
        }
    }

    // ── 5. ONNX inference — Phase 1 ──────────────────────────────────────────
    let input_fast = Tensor::from_array(tensor_fast).map_err(|e| {
        AppError::Internal(format!("Failed to build Phase 1 input tensor: {}", e))
    })?;

    // Load Phase 1 model on-demand to conserve memory on low-resource environments (e.g. Render)
    let mut session_fast = load_onnx_session("assets/u2netp.onnx")?;
    let result_fast = session_fast
        .run(ort::inputs![input_fast])
        .map_err(|e| AppError::Internal(format!("Phase 1 inference failed: {}", e)))?;

    // ── 6. Extract Phase 1 mask ───────────────────────────────────────────────
    let out_fast: &ort::value::DynValue = if result_fast.get("output.0").is_some() {
        result_fast.get("output.0").unwrap()
    } else if result_fast.len() > 0 {
        &result_fast[0usize]
    } else {
        return Err(AppError::Internal("Phase 1 model returned no outputs.".to_string()));
    };

    let out_fast_tensor = out_fast
        .downcast_ref::<ort::value::DynTensorValueType>()
        .map_err(|e| AppError::Internal(format!("Failed to downcast Phase 1 output: {}", e)))?;

    let mask_fast_view = out_fast_tensor
        .try_extract_array::<f32>()
        .map_err(|e| AppError::Internal(format!("Failed to read Phase 1 output array: {}", e)))?;

    // ── 7. Build rough mask — copy pixels into owned buffer ─────────────────
    let mut rough_mask: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(320, 320);
    for y in 0..320u32 {
        for x in 0..320u32 {
            let prob = mask_fast_view[[0, 0, y as usize, x as usize]].clamp(0.0, 1.0);
            rough_mask.put_pixel(x, y, Luma([(prob * 255.0) as u8]));
        }
    }
    // result_fast borrows session_fast — drop them explicitly to release RAM immediately.
    drop(result_fast);
    drop(session_fast);

    let rough_mask_resized = DynamicImage::ImageLuma8(rough_mask)
        .resize_exact(original_width, original_height, image::imageops::FilterType::Triangle)
        .to_luma8();

    // ── 8. Intermediate composite — white-background RGB for Phase 2 ──────────
    // Pixels below the rough-mask threshold get replaced with white so that
    // RMBG-1.4 receives a scene with most background already neutralised.
    // We keep the original image data for pixels the rough mask considers
    // foreground, preserving colour fidelity for the refined pass.
    let orig_rgb = original_img.to_rgb8();
    let mut preclean: RgbImage = ImageBuffer::new(original_width, original_height);
    for y in 0..original_height {
        for x in 0..original_width {
            let alpha = rough_mask_resized.get_pixel(x, y)[0];
            if alpha >= 128 {
                preclean.put_pixel(x, y, *orig_rgb.get_pixel(x, y));
            } else {
                preclean.put_pixel(x, y, Rgb([255u8, 255, 255]));
            }
        }
    }

    // ── 9. PHASE 2: RMBG-1.4 — refined cleanup (1024×1024, mean=0.5/std=1.0) ─
    let preclean_dyn = DynamicImage::ImageRgb8(preclean);
    let resized_refined = preclean_dyn
        .resize_exact(1024, 1024, image::imageops::FilterType::Triangle);
    let rgb_refined = resized_refined.to_rgb8();

    let mean_refined = [0.5f32, 0.5, 0.5];
    let std_refined  = [1.0f32, 1.0, 1.0];

    let mut tensor_refined = ndarray::Array4::<f32>::zeros((1, 3, 1024, 1024));
    for y in 0..1024usize {
        for x in 0..1024usize {
            let pixel = rgb_refined.get_pixel(x as u32, y as u32);
            for c in 0..3usize {
                tensor_refined[[0, c, y, x]] =
                    (pixel[c] as f32 / 255.0 - mean_refined[c]) / std_refined[c];
            }
        }
    }

    // ── 10. ONNX inference — Phase 2 ─────────────────────────────────────────
    let input_refined = Tensor::from_array(tensor_refined).map_err(|e| {
        AppError::Internal(format!("Failed to build Phase 2 input tensor: {}", e))
    })?;

    // Load Phase 2 model on-demand to conserve memory on low-resource environments (e.g. Render)
    let mut session_refined = load_onnx_session("assets/rmbg-1.4.onnx")?;
    let result_refined = session_refined
        .run(ort::inputs![input_refined])
        .map_err(|e| AppError::Internal(format!("Phase 2 inference failed: {}", e)))?;

    // ── 11. Extract Phase 2 output tensor ────────────────────────────────────
    let out_refined: &ort::value::DynValue = if result_refined.get("output.0").is_some() {
        result_refined.get("output.0").unwrap()
    } else if result_refined.len() > 0 {
        &result_refined[0usize]
    } else {
        return Err(AppError::Internal("Phase 2 model returned no outputs.".to_string()));
    };

    let out_refined_tensor = out_refined
        .downcast_ref::<ort::value::DynTensorValueType>()
        .map_err(|e| AppError::Internal(format!("Failed to downcast Phase 2 output: {}", e)))?;

    let mask_refined_view = out_refined_tensor
        .try_extract_array::<f32>()
        .map_err(|e| AppError::Internal(format!("Failed to read Phase 2 output array: {}", e)))?;

    // ── 12. Postprocess Phase 2 mask — min-max normalise ─────────────────────
    // RMBG-1.4 outputs raw logits — must be min-max normalised before use.
    let mask_slice = mask_refined_view.as_slice().unwrap_or_default();
    let raw_vals: Vec<f32> = mask_slice.iter().take(1024 * 1024).copied().collect();
    let min_val = raw_vals.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_val = raw_vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range   = (max_val - min_val).max(1e-6);

    let mut refined_mask_img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(1024, 1024);
    for y in 0..1024u32 {
        for x in 0..1024u32 {
            let raw = mask_refined_view[[0, 0, y as usize, x as usize]];
            let normalised = ((raw - min_val) / range).clamp(0.0, 1.0);
            refined_mask_img.put_pixel(x, y, Luma([(normalised * 255.0) as u8]));
        }
    }
    // result_refined borrows session_refined — drop them explicitly to release RAM immediately.
    drop(result_refined);
    drop(session_refined);

    let refined_mask_resized = DynamicImage::ImageLuma8(refined_mask_img)
        .resize_exact(original_width, original_height, image::imageops::FilterType::Triangle)
        .to_luma8();

    // ── 13. Alpha compositing — apply refined mask to ORIGINAL image ──────────
    // Always composite on the original (not the pre-cleaned copy) to keep
    // the full original colour and detail in the output.
    let mut rgba_img = original_img.to_rgba8();
    for y in 0..original_height {
        for x in 0..original_width {
            rgba_img.get_pixel_mut(x, y)[3] = refined_mask_resized.get_pixel(x, y)[0];
        }
    }

    // ── 14. Encode to PNG ─────────────────────────────────────────────────────
    let mut output_bytes = Vec::new();
    rgba_img
        .write_to(
            &mut std::io::Cursor::new(&mut output_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| AppError::Internal(format!("Failed to encode output PNG: {}", e)))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/png")],
        output_bytes,
    ))
}


