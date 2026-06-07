# G-Remover Backend API

A high-performance, modular backend API built with Rust using the [Axum](https://github.com/tokio-rs/axum) web framework and [Tokio](https://tokio.rs/) async runtime.

## Tech Stack
- **Framework**: Axum (v0.7)
- **Async Runtime**: Tokio
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
│   ├── main.rs        # Application setup, middleware attachment, server boot
│   ├── middleware/    # CORS policies and network logging layers
│   │   └── mod.rs
│   └── routes/        # Router configuration and API handlers
│       ├── mod.rs
│       └── welcome.html # landing portal page
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
   Edit `.env` if you wish to change host/port settings:
   ```env
   HOST=127.0.0.1
   PORT=8080
   RUST_LOG=backend=debug,tower_http=debug,axum=debug
   ```

3. **Run in Development**:
   ```bash
   cargo run
   ```

4. **Verify API Endpoints**:
   - Web Landing Portal: `http://127.0.0.1:8080/`
   - Liveness Check: `http://127.0.0.1:8080/api/health`
   - Metadata / Info: `http://127.0.0.1:8080/api/info`

---

## API Endpoints

### 1. `GET /`
Serves a responsive landing page with full links, styling, and server-active states.

### 2. `GET /api/health`
Checks whether the service is alive and reachable.
**Response (JSON)**:
```json
{
  "status": "ok",
  "timestamp": 1716382000,
  "service": "g-remover-backend"
}
```

### 3. `GET /api/info`
Returns general application metadata, framework, runtime environment, and available routes.
**Response (JSON)**:
```json
{
  "app_name": "G-Remover API",
  "version": "0.1.0",
  "framework": "Axum 0.7",
  "runtime": "Tokio",
  "status": "operational",
  "endpoints": [...]
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
