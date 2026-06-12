# ==========================================
# Stage 1: Build
# ==========================================
FROM rust:alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev pkgconfig netcdf-dev hdf5-dev gcc g++ make

WORKDIR /usr/src/nimbus

ENV RUSTFLAGS="-C target-feature=-crt-static"

# Copy manifest and code files
COPY Cargo.toml ./
COPY src/ ./src/

# Compile the release binary
RUN cargo build --release

# ==========================================
# Stage 2: Runtime
# ==========================================
FROM alpine:latest

# Install runtime dependencies (NetCDF and HDF5 C libraries, TLS certificates, GCC runtime libraries)
RUN apk add --no-cache netcdf hdf5 ca-certificates libgcc libstdc++

WORKDIR /app

# Copy release binary and static assets
COPY --from=builder /usr/src/nimbus/target/release/weer-service /app/weer-service
COPY static/ /app/static/

EXPOSE 8080

CMD ["./weer-service"]
