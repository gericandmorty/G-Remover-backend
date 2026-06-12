use axum::{
    extract::{Multipart, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use image::{DynamicImage, ImageBuffer, Luma};
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
            let declared_content_type: Option<String> = field
                .content_type()
                .map(|ct| ct.to_string());

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

    let original_width  = original_img.width();
    let original_height = original_img.height();

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

    // ── 4. Preprocess for RMBG-1.4 (1024×1024, mean=0.5 / std=1.0) ──────────
    let resized = original_img.resize_exact(1024, 1024, image::imageops::FilterType::Triangle);
    let rgb     = resized.to_rgb8();

    let mean = [0.5f32, 0.5, 0.5];
    let std  = [1.0f32, 1.0, 1.0];

    let mut tensor = ndarray::Array4::<f32>::zeros((1, 3, 1024, 1024));
    for y in 0..1024usize {
        for x in 0..1024usize {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            for c in 0..3usize {
                tensor[[0, c, y, x]] = (pixel[c] as f32 / 255.0 - mean[c]) / std[c];
            }
        }
    }

    // ── 5. ONNX inference ─────────────────────────────────────────────────────
    let input = Tensor::from_array(tensor).map_err(|e| {
        AppError::Internal(format!("Failed to build input tensor: {}", e))
    })?;

    let mut session = state.model.lock().await;
    let result = session
        .run(ort::inputs![input])
        .map_err(|e| AppError::Internal(format!("Inference failed: {}", e)))?;

    // ── 6. Extract output tensor ──────────────────────────────────────────────
    let out: &ort::value::DynValue = if result.get("output.0").is_some() {
        result.get("output.0").unwrap()
    } else if result.len() > 0 {
        &result[0usize]
    } else {
        return Err(AppError::Internal("Model returned no outputs.".to_string()));
    };

    let out_tensor = out
        .downcast_ref::<ort::value::DynTensorValueType>()
        .map_err(|e| AppError::Internal(format!("Failed to downcast output: {}", e)))?;

    let mask_view = out_tensor
        .try_extract_array::<f32>()
        .map_err(|e| AppError::Internal(format!("Failed to read output array: {}", e)))?;

    // ── 7. Min-max normalise the raw logits ───────────────────────────────────
    let mask_slice = mask_view.as_slice().unwrap_or_default();
    let raw_vals: Vec<f32> = mask_slice.iter().take(1024 * 1024).copied().collect();
    let min_val = raw_vals.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_val = raw_vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range   = (max_val - min_val).max(1e-6);

    let mut mask_img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(1024, 1024);
    for y in 0..1024u32 {
        for x in 0..1024u32 {
            let raw        = mask_view[[0, 0, y as usize, x as usize]];
            let normalised = ((raw - min_val) / range).clamp(0.0, 1.0);
            mask_img.put_pixel(x, y, Luma([(normalised * 255.0) as u8]));
        }
    }

    // Release session lock and inference output before compositing
    drop(result);
    drop(session);

    // ── 8. Resize mask back to original dimensions ────────────────────────────
    let mask_resized = DynamicImage::ImageLuma8(mask_img)
        .resize_exact(original_width, original_height, image::imageops::FilterType::Triangle)
        .to_luma8();

    // ── 9. Alpha composite — apply mask to original image ────────────────────
    let mut rgba_img = original_img.to_rgba8();
    for y in 0..original_height {
        for x in 0..original_width {
            rgba_img.get_pixel_mut(x, y)[3] = mask_resized.get_pixel(x, y)[0];
        }
    }

    // ── 10. Encode to PNG ─────────────────────────────────────────────────────
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
