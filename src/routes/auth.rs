use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{encode, EncodingKey, Header};
use mongodb::bson::{doc, DateTime};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::{AppError, Result};
use crate::models::user::{Claims, LoginPayload, RegisterPayload, TokenResponse, User};
use crate::state::AppState;

// POST /api/auth/register
pub async fn register_handler(
    State(state): State<AppState>,
    Json(payload): Json<RegisterPayload>,
) -> Result<impl IntoResponse> {
    // Validate inputs
    if payload.email.trim().is_empty() {
        return Err(AppError::BadRequest("Email cannot be empty".to_string()));
    }

    let trimmed_password = payload.password.trim();

    if trimmed_password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters long".to_string(),
        ));
    }
    if !trimmed_password.chars().any(|c| c.is_uppercase()) {
        return Err(AppError::BadRequest(
            "Password must contain at least one uppercase letter".to_string(),
        ));
    }
    if !trimmed_password.chars().any(|c| c.is_numeric()) {
        return Err(AppError::BadRequest(
            "Password must contain at least one number".to_string(),
        ));
    }
    if !trimmed_password.chars().any(|c| !c.is_alphanumeric()) {
        return Err(AppError::BadRequest(
            "Password must contain at least one special character".to_string(),
        ));
    }

    let collection = state.db.collection::<User>("users");

    // Check if user already exists
    let filter = doc! { "email": &payload.email };
    let existing_user = collection.find_one(filter).await?;
    if existing_user.is_some() {
        return Err(AppError::Conflict("A user with this email already exists".to_string()));
    }

    // Hash password with bcrypt
    let password_hash = hash(trimmed_password, DEFAULT_COST)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    // Create user document
    let new_user = User {
        id: None,
        email: payload.email.trim().to_string(),
        password: password_hash,
        created_at: DateTime::now(),
    };

    // Insert user into MongoDB
    collection.insert_one(new_user).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "status": "success",
            "message": "User registered successfully"
        })),
    ))
}

// POST /api/auth/login
pub async fn login_handler(
    State(state): State<AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<impl IntoResponse> {
    // Validate inputs
    let trimmed_password = payload.password.trim();
    if payload.email.trim().is_empty() || trimmed_password.is_empty() {
        return Err(AppError::BadRequest("Email and password are required".to_string()));
    }

    let collection = state.db.collection::<User>("users");

    // Find user by email
    let filter = doc! { "email": payload.email.trim() };
    let user = collection.find_one(filter).await?
        .ok_or_else(|| AppError::Unauthorized("Invalid email or password".to_string()))?;

    // Verify password hash
    let is_valid = verify(trimmed_password, &user.password)
        .map_err(|e| AppError::Internal(format!("Failed to verify password hash: {}", e)))?;

    if !is_valid {
        return Err(AppError::Unauthorized("Invalid email or password".to_string()));
    }

    // Generate JWT token (expires in 24 hours)
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize;
    let expiration = now + 24 * 3600;

    let claims = Claims {
        sub: user.id.map(|id| id.to_hex()).unwrap_or_else(|| user.email.clone()),
        email: user.email.clone(),
        exp: expiration,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("Failed to generate auth token: {}", e)))?;

    Ok((
        StatusCode::OK,
        Json(TokenResponse {
            token,
            token_type: "Bearer".to_string(),
        }),
    ))
}
