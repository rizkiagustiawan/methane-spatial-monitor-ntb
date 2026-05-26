# Builder Stage
FROM rust:1.75-bookworm as builder

# Install GDAL and other build dependencies
RUN apt-get update && apt-get install -y \
    libgdal-dev \
    pkg-config \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY . .

# Build for release
RUN cargo build --release

# Runtime Stage
FROM debian:bookworm-slim

# Install runtime dependencies for GDAL and SSL
RUN apt-get update && apt-get install -y \
    libgdal32 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/bin
COPY --from=builder /usr/src/app/target/release/geoesg-aeco-backend .
COPY --from=builder /usr/src/app/ntb_dem.tif .

# Expose the API port
EXPOSE 3000

# Run the binary
CMD ["./geoesg-aeco-backend"]
