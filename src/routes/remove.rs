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

// POST /api/v1/remove-background
pub async fn remove_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<impl IntoResponse> {
    // 1. Authorize Request (Optional)
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()) {
        if auth_header.starts_with("Bearer ") && auth_header.len() > 7 {
            let token = &auth_header[7..];
            if token != "undefined" && token != "null" && !token.is_empty() {
                // Decode and validate token
                let _claims = decode::<Claims>(
                    token,
                    &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
                    &Validation::default(),
                )
                .map_err(|_| AppError::Unauthorized("Invalid or expired authorization token".to_string()))?
                .claims;
            }
        }
    }

    // 2. Parse Multipart File Upload
    let mut multipart = multipart;
    let mut image_bytes = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::BadRequest(format!("Failed to parse multipart field: {}", e))
    })? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "image" {
            let data = field.bytes().await.map_err(|e| {
                AppError::BadRequest(format!("Failed to read image field bytes: {}", e))
            })?;

            // Enforce strict size limit (10MB)
            if data.len() > 10 * 1024 * 1024 {
                return Err(AppError::BadRequest("File size exceeds 10MB limit".to_string()));
            }

            image_bytes = Some(data.to_vec());
            break;
        }
    }

    let raw_bytes = image_bytes.ok_or_else(|| {
        AppError::BadRequest("No image field found in multipart request".to_string())
    })?;

    // 3. Load Image
    let original_img = image::load_from_memory(&raw_bytes).map_err(|e| {
        AppError::BadRequest(format!("Invalid image file format: {}", e))
    })?;

    let original_width = original_img.width();
    let original_height = original_img.height();

    // 4. Preprocess Image (Resize to 320x320 & Normalization)
    let resized_img = original_img.resize_exact(320, 320, image::imageops::FilterType::Triangle);
    let rgb = resized_img.to_rgb8();

    // Normalization constants (ImageNet mean & std)
    let mean = [0.485f32, 0.456, 0.406];
    let std_dev = [0.229f32, 0.224, 0.225];

    let mut tensor_data = ndarray::Array4::<f32>::zeros((1, 3, 320, 320));
    for y in 0..320usize {
        for x in 0..320usize {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            for c in 0..3usize {
                let val = (pixel[c] as f32 / 255.0 - mean[c]) / std_dev[c];
                tensor_data[[0, c, y, x]] = val;
            }
        }
    }

    // 5. Run ONNX Model Inference
    // Convert ndarray -> ort Tensor
    let input_tensor = Tensor::from_array(tensor_data).map_err(|e| {
        AppError::Internal(format!("Failed to create input tensor: {}", e))
    })?;

    // Lock the session (required because Session::run takes &mut self)
    let mut session = state.model.lock().await;

    let result = session
        .run(ort::inputs![input_tensor])
        .map_err(|e| AppError::Internal(format!("Model execution failure: {}", e)))?;

    // 6. Extract output tensor - try by name first, fallback to first output by index
    let has_named = result.get("output.0").is_some();
    let output_dyn: &ort::value::DynValue = if has_named {
        result.get("output.0").unwrap()
    } else if result.len() > 0 {
        &result[0usize]
    } else {
        return Err(AppError::Internal("Model returned no outputs".to_string()));
    };

    // Downcast DynValue -> DynTensor, then extract as f32 ndarray view
    let output_tensor = output_dyn
        .downcast_ref::<ort::value::DynTensorValueType>()
        .map_err(|e| AppError::Internal(format!("Failed to downcast model output: {}", e)))?;

    let mask_view = output_tensor
        .try_extract_array::<f32>()
        .map_err(|e| AppError::Internal(format!("Failed to extract output array: {}", e)))?;

    // 7. Postprocess Inference Mask
    // Map probability values back to 320x320 grayscale Luma buffer
    let mut mask_img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::new(320, 320);
    for y in 0..320u32 {
        for x in 0..320u32 {
            let prob = mask_view[[0, 0, y as usize, x as usize]];
            let prob_clamped = prob.clamp(0.0, 1.0);
            let val = (prob_clamped * 255.0) as u8;
            mask_img.put_pixel(x, y, Luma([val]));
        }
    }

    // Resize grayscale mask back to match original image dimensions
    let resized_mask = DynamicImage::ImageLuma8(mask_img).resize_exact(
        original_width,
        original_height,
        image::imageops::FilterType::Triangle,
    );
    let mask_luma = resized_mask.to_luma8();

    // 8. Apply Alpha Compositing
    let mut rgba_img = original_img.to_rgba8();
    for y in 0..original_height {
        for x in 0..original_width {
            let mask_pixel = mask_luma.get_pixel(x, y);
            let alpha = mask_pixel[0];

            let pixel = rgba_img.get_pixel_mut(x, y);
            pixel[3] = alpha; // Apply transparency to alpha channel
        }
    }

    // 9. Encode Result to PNG bytes
    let mut output_bytes = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut output_bytes);
    rgba_img
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| AppError::Internal(format!("Failed to encode output PNG: {}", e)))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/png")],
        output_bytes,
    ))
}
