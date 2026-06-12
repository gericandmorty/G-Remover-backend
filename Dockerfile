# Multi-stage build for G-Remover Backend using Ubuntu 24.04 to support GLIBC 2.38+
FROM ubuntu:24.04 AS builder

# Install build dependencies
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install Rust toolchain
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /usr/src/app

# Copy Cargo files first to leverage Docker layer caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build and cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src/ target/release/deps/backend*

# Copy assets folder (which contains .gitkeep to ensure the directory is created)
COPY assets ./assets

# Download the quantized RMBG-1.4 model (~42 MB) if not already present
RUN if [ ! -f assets/rmbg-1.4.onnx ]; then \
        echo "Downloading RMBG-1.4 quantized model..." && \
        curl -L -o assets/rmbg-1.4.onnx "https://huggingface.co/briaai/RMBG-1.4/resolve/main/onnx/model_quantized.onnx"; \
    fi

# Copy the actual source code
COPY src ./src

# Build the real binary
RUN cargo build --release

# Stage 2: Runtime stage (Ubuntu 24.04 to match GLIBC version)
FROM ubuntu:24.04

# Install runtime dependencies
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl-dev \
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
