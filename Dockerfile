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

# Download the U2Netp model if it's not present (e.g. when building on Render where it is gitignored)
RUN if [ ! -f assets/u2netp.onnx ]; then \
        curl -L -o assets/u2netp.onnx https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx; \
    fi

# Model is assumed to be in the assets folder
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
ENV OMP_NUM_THREADS=1
ENV MKL_NUM_THREADS=1
ENV OPENBLAS_NUM_THREADS=1
ENV VECLIB_MAXIMUM_THREADS=1
ENV NUMEXPR_NUM_THREADS=1

EXPOSE 8080

CMD ["/app/backend"]
