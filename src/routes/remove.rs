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
    let mut background_color: String = "transparent".to_string();

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
        } else if name == "background_color" {
            if let Ok(value) = field.text().await {
                background_color = value.trim().to_lowercase();
            }
        }
    }

    let raw_bytes = image_bytes.ok_or_else(|| {
        AppError::BadRequest(
            "No 'image' field found in the request. \
             Send the file as multipart/form-data with field name 'image'."
                .to_string(),
        )
    })?;

    // ── 3. Check dimensions BEFORE fully decoding to prevent decompression bombs
    let reader = image::ImageReader::new(std::io::Cursor::new(&raw_bytes))
        .with_guessed_format()
        .map_err(|e| AppError::BadRequest(format!("Failed to guess image format: {}", e)))?;

    let (width, height) = reader.into_dimensions().map_err(|e| {
        AppError::UnprocessableEntity(format!("Failed to read image dimensions: {}", e))
    })?;

    if width < 4 || height < 4 {
        return Err(AppError::UnprocessableEntity(
            "Image dimensions are too small. Minimum size is 4×4 pixels.".to_string(),
        ));
    }
    if width > 2048 || height > 2048 {
        return Err(AppError::UnprocessableEntity(
            "Image dimensions exceed the 2048×2048 pixel limit. \
             Please downscale the image before uploading."
                .to_string(),
        ));
    }

    // ── 4. Safely decode the image now that dimensions are validated ──────────
    let original_img = image::load_from_memory(&raw_bytes).map_err(|e| {
        tracing::warn!("Image decode failed: {}", e);
        AppError::UnprocessableEntity(
            "Could not decode the uploaded image. The file may be corrupt or truncated."
                .to_string(),
        )
    })?;

    let original_width = width;
    let original_height = height;

    // ── 4. Preprocess for U2Netp (320x320, max normalization) ──────────
    let resized = original_img.resize_exact(320, 320, image::imageops::FilterType::Triangle);
    let rgb     = resized.into_rgb8();

    let mut tensor = ndarray::Array4::<f32>::zeros((1, 3, 320, 320));
    
    // U2Netp normalization: (pixel / 255.0 - 0.485) / 0.229 (approximate ImageNet or max division)
    // The previous U2Netp implementation used specific mean/std or min-max normalization. 
    // Here we use standard ImageNet mean/std which works for most pre-trained models.
    let mean = [0.485f32, 0.456, 0.406];
    let std  = [0.229f32, 0.224, 0.225];

    for y in 0..320usize {
        for x in 0..320usize {
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

    // ── 7. Normalise and extract the mask ──────────────────
    let mut mask_img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(320, 320);
    
    // Find min and max for normalization
    let mut min_val = f32::MAX;
    let mut max_val = f32::MIN;
    for y in 0..320u32 {
        for x in 0..320u32 {
            let val = mask_view[[0, 0, y as usize, x as usize]];
            if val < min_val { min_val = val; }
            if val > max_val { max_val = val; }
        }
    }

    let range = if max_val - min_val < 1e-5 { 1.0 } else { max_val - min_val };

    for y in 0..320u32 {
        for x in 0..320u32 {
            let val = mask_view[[0, 0, y as usize, x as usize]];
            let normalized = (val - min_val) / range;
            mask_img.put_pixel(x, y, Luma([(normalized * 255.0) as u8]));
        }
    }

    // Release session lock and inference output before compositing
    drop(result);
    drop(session);

    // ── 8. Resize mask back to original dimensions ────────────────────────────
    let mask_resized = DynamicImage::ImageLuma8(mask_img)
        .resize_exact(original_width, original_height, image::imageops::FilterType::Triangle)
        .to_luma8();

    // ── 9. Compositing — apply mask to original image with selected bg ─────────
    let mut rgba_img = original_img.into_rgba8();
    for y in 0..original_height {
        for x in 0..original_width {
            let mask_val = mask_resized.get_pixel(x, y)[0] as f32 / 255.0;
            let orig_px = rgba_img.get_pixel(x, y);

            if background_color == "white" {
                // Blend color with solid white, set alpha to 255 (fully opaque)
                let r = (orig_px[0] as f32 * mask_val + 255.0 * (1.0 - mask_val)) as u8;
                let g = (orig_px[1] as f32 * mask_val + 255.0 * (1.0 - mask_val)) as u8;
                let b = (orig_px[2] as f32 * mask_val + 255.0 * (1.0 - mask_val)) as u8;
                rgba_img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
            } else if background_color == "green" {
                // Blend color with solid green, set alpha to 255 (fully opaque)
                let r = (orig_px[0] as f32 * mask_val + 0.0 * (1.0 - mask_val)) as u8;
                let g = (orig_px[1] as f32 * mask_val + 255.0 * (1.0 - mask_val)) as u8;
                let b = (orig_px[2] as f32 * mask_val + 0.0 * (1.0 - mask_val)) as u8;
                rgba_img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
            } else {
                // Transparent background - apply mask value directly as alpha channel
                rgba_img.get_pixel_mut(x, y)[3] = mask_resized.get_pixel(x, y)[0];
            }
        }
    }

    // Drop the mask buffer early to free up memory before PNG encoding
    drop(mask_resized);

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
