use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use bcrypt::{hash, verify, DEFAULT_COST};
use jsonwebtoken::{encode, EncodingKey, Header};
use mongodb::bson::{doc, DateTime};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::{AppError, Result};
use crate::models::user::{Claims, LoginPayload, RegisterPayload, TokenResponse, User};
use crate::state::AppState;

/// Very basic email format check: must contain exactly one @, a dot after it,
/// and non-empty local/domain parts.
fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

// POST /api/auth/register
pub async fn register_handler(
    State(state): State<AppState>,
    Json(payload): Json<RegisterPayload>,
) -> Result<impl IntoResponse> {

    let email = payload.email.trim().to_lowercase();
    let password = payload.password.trim();

    // ── Email validation ───────────────────────────────────────────────────────
    if email.is_empty() {
        return Err(AppError::BadRequest("Email is required.".to_string()));
    }
    if !is_valid_email(&email) {
        return Err(AppError::BadRequest(
            "Email address is not valid. Please enter a correctly formatted email.".to_string(),
        ));
    }

    // ── Password rules ─────────────────────────────────────────────────────────
    if password.is_empty() {
        return Err(AppError::BadRequest("Password is required.".to_string()));
    }
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters long.".to_string(),
        ));
    }
    if !password.chars().any(|c| c.is_uppercase()) {
        return Err(AppError::BadRequest(
            "Password must contain at least one uppercase letter.".to_string(),
        ));
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "Password must contain at least one number.".to_string(),
        ));
    }
    if !password.chars().any(|c| !c.is_alphanumeric()) {
        return Err(AppError::BadRequest(
            "Password must contain at least one special character (e.g. !, @, #, $).".to_string(),
        ));
    }

    let collection = state.db.collection::<User>("users");

    // ── Duplicate check ────────────────────────────────────────────────────────
    let existing = collection
        .find_one(doc! { "email": &email })
        .await
        .map_err(|e| AppError::Internal(format!("Database lookup failed: {}", e)))?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "An account with this email already exists.".to_string(),
        ));
    }

    // ── Hash & persist ─────────────────────────────────────────────────────────
    let password_hash = hash(password, DEFAULT_COST)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?;

    collection
        .insert_one(User {
            id: None,
            email: email.clone(),
            password: password_hash,
            created_at: DateTime::now(),
        })
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create user: {}", e)))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "status": "success",
            "message": "Account created successfully."
        })),
    ))
}

// POST /api/auth/login
pub async fn login_handler(
    State(state): State<AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<impl IntoResponse> {

    let email = payload.email.trim().to_lowercase();
    let password = payload.password.trim();

    // ── Input presence checks ──────────────────────────────────────────────────
    if email.is_empty() {
        return Err(AppError::BadRequest("Email is required.".to_string()));
    }
    if password.is_empty() {
        return Err(AppError::BadRequest("Password is required.".to_string()));
    }

    let collection = state.db.collection::<User>("users");

    // ── Lookup user — use a generic error to avoid account enumeration ─────────
    let user = collection
        .find_one(doc! { "email": &email })
        .await
        .map_err(|e| AppError::Internal(format!("Database lookup failed: {}", e)))?
        .ok_or_else(|| AppError::Unauthorized("Invalid email or password.".to_string()))?;

    // ── Verify password ────────────────────────────────────────────────────────
    let is_valid = verify(password, &user.password)
        .map_err(|e| AppError::Internal(format!("Failed to verify password: {}", e)))?;

    if !is_valid {
        return Err(AppError::Unauthorized("Invalid email or password.".to_string()));
    }

    // ── Issue JWT (24-hour expiry) ─────────────────────────────────────────────
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize;

    let claims = Claims {
        sub: user.id.map(|id| id.to_hex()).unwrap_or_else(|| user.email.clone()),
        email: user.email.clone(),
        exp: now + 24 * 3600,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("Failed to generate token: {}", e)))?;

    Ok((
        StatusCode::OK,
        Json(TokenResponse {
            token,
            token_type: "Bearer".to_string(),
        }),
    ))
}
