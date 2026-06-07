# Multi-stage build for G-Remover Backend
FROM rust:slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    ca-certificates \
    g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Copy Cargo files first to leverage Docker layer caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build and cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src/ target/release/deps/backend*

# Copy the actual source code and assets
COPY src ./src
COPY assets ./assets

# Build the real binary
RUN cargo build --release

# Stage 2: Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    openssl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the compiled binary and ONNX shared libraries from builder stage
COPY --from=builder /usr/src/app/target/release/backend /app/backend
COPY --from=builder /usr/src/app/target/release/libonnxruntime.so* /app/

# Copy the ONNX model asset
COPY --from=builder /usr/src/app/assets /app/assets

# Expose the API port and configure runtime environment
ENV HOST=0.0.0.0
ENV PORT=8080
ENV LD_LIBRARY_PATH=/app

EXPOSE 8080

CMD ["/app/backend"]
