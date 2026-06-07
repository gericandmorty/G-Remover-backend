use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

// Database model representation for MongoDB
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub email: String,
    pub password: String,
    pub created_at: mongodb::bson::DateTime,
}

// Struct to parse registration input request body
#[derive(Debug, Deserialize)]
pub struct RegisterPayload {
    pub email: String,
    pub password: String,
}

// Struct to parse login input request body
#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
}

// Struct to format successful login response
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub token: String,
    pub token_type: String,
}

// JWT Claims representation
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // User email or object ID
    pub email: String,
    pub exp: usize,  // Expiration timestamp in seconds since epoch
}
