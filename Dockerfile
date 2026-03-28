FROM rust:1.94-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies: copy manifests, create dummy src, build deps only.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --no-default-features --features server 2>/dev/null || true

# Now copy real source and build.
COPY src/ src/
COPY dashboard/ dashboard/
COPY migrations/ migrations/
RUN touch src/main.rs && cargo build --release --no-default-features --features server

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/walker /usr/local/bin/walker

EXPOSE 3000

CMD ["walker", "listen", "--port", "3000"]
