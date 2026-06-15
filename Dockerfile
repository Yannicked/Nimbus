# ==========================================
# Stage 1: Cargo Chef - Base & Planner
# ==========================================
FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
WORKDIR /usr/src/nimbus

# Install system dependencies required for compilation
RUN apk add --no-cache musl-dev pkgconfig netcdf-dev hdf5-dev gcc g++ make

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ==========================================
# Stage 2: Builder - Cook dependencies & Build app
# ==========================================
FROM chef AS builder
ENV RUSTFLAGS="-C target-feature=-crt-static"

# Copy recipe and cook dependencies
COPY --from=planner /usr/src/nimbus/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy actual source code and compile the application
COPY . .
RUN cargo build --release

# ==========================================
# Stage 3: Runtime
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
