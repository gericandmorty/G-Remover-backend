# G-Remover Backend API

A high-performance, modular backend API built with Rust using the [Axum](https://github.com/tokio-rs/axum) web framework, [Tokio](https://tokio.rs/) async runtime, and [MongoDB](https://www.mongodb.com/) database storage.

## Tech Stack
- **Framework**: Axum (v0.7)
- **Async Runtime**: Tokio
- **Database**: MongoDB
- **Authentication**: Bcrypt password hashing & JSON Web Tokens (JWT)
- **Logging**: Tracing & Tracing-Subscriber
- **Configuration**: Dotenvy
- **CORS/Request Logging**: Tower & Tower-HTTP
- **Error Handling**: Thiserror

---

## Directory Structure
```text
backend/
├── src/
│   ├── config.rs      # Environment variables configuration loader
│   ├── errors.rs      # Centralized error types and JSON API response mappings
│   ├── main.rs        # Application setup, DB connection, server boot
│   ├── state.rs       # Shared AppState struct (holds DB connection & JWT secret)
│   ├── middleware/    # CORS policies and network logging layers
│   │   └── mod.rs
│   ├── models/        # Database document models
│   │   ├── mod.rs
│   │   └── user.rs
│   └── routes/        # Router configuration and API handlers
│       ├── mod.rs
│       └── auth.rs    # User registration and login handlers
├── .env               # Local environment settings
└── Cargo.toml         # Dependency configurations
```

---

## Getting Started

### Prerequisites
Make sure Rust and Cargo are installed. If not, install them using:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Installation & Run

1. **Clone/Navigate to project**:
   ```bash
   cd backend
   ```

2. **Configure environment**:
   Create a `.env` file (which is ignored by Git via `.gitignore`):
   ```env
   HOST=127.0.0.1
   PORT=8080
   RUST_LOG=backend=debug,tower_http=debug,axum=debug
   MONGODB_URI=mongodb+srv://...
   MONGODB_DB_NAME=g_remover
   JWT_SECRET=your_jwt_secret_key
   ```

3. **Run in Development**:
   ```bash
   cargo run
   ```

4. **Verify API Endpoints**:
   - Liveness Check: `http://127.0.0.1:8080/api/health`
   - Metadata / Info: `http://127.0.0.1:8080/api/info`

---

## API Endpoints

### 1. `GET /api/health`
Checks whether the service is alive and reachable.
**Response (JSON)**:
```json
{
  "status": "ok",
  "timestamp": 1716382000,
  "service": "g-remover-backend"
}
```

### 2. `GET /api/info`
Returns general application metadata, framework, runtime environment, and available routes.

### 3. `POST /api/auth/register`
Creates a new user profile with password encryption (Bcrypt).
**Request Body (JSON)**:
```json
{
  "email": "user@example.com",
  "password": "securepassword123"
}
```
**Response (201 Created)**:
```json
{
  "status": "success",
  "message": "User registered successfully"
}
```

### 4. `POST /api/auth/login`
Validates user credentials and issues a signed JSON Web Token (JWT).
**Request Body (JSON)**:
```json
{
  "email": "user@example.com",
  "password": "securepassword123"
}
```
**Response (200 OK)**:
```json
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer"
}
```

---

## Testing & Compiling

- **Check compilation**:
  ```bash
  cargo check
  ```
- **Run test suites**:
  ```bash
  cargo test
  ```
- **Build production bundle**:
  ```bash
  cargo build --release
  ```
