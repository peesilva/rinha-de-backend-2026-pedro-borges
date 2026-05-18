# ─── Stage 1: Build the Rust binaries ───────────────────────────────────────
FROM rust:1.82-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src && \
    echo "pub fn dummy() {}" > src/lib.rs && \
    echo 'fn main() {}' > src/build_index.rs && \
    echo 'fn main() {}' > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -f src/lib.rs src/build_index.rs src/main.rs

COPY src/ src/

ENV RUSTFLAGS="-C target-cpu=x86-64-v2"
RUN cargo build --release --bin build-index --bin server

# ─── Stage 2: Build VP-Tree index ───────────────────────────────────────────
FROM debian:bookworm-slim AS indexer

RUN apt-get update && apt-get install -y --no-install-recommends \
    wget ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/build-index /usr/local/bin/build-index

ARG REFERENCES_URL=https://github.com/zanfranceschi/rinha-de-backend-2026/raw/main/resources/references.json.gz

RUN mkdir -p /data && \
    echo "[indexer] Downloading references..." && \
    wget -q -O /tmp/references.json.gz "${REFERENCES_URL}" && \
    echo "[indexer] Building VP-Tree index..." && \
    build-index /tmp/references.json.gz /data/index.bin && \
    rm /tmp/references.json.gz && \
    echo "[indexer] Done."

# ─── Stage 3: Final runtime image ────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/server /usr/local/bin/server
COPY --from=indexer /data/index.bin /data/index.bin

RUN useradd -r -s /bin/false appuser
USER appuser

ENV PORT=8080
ENV INDEX_PATH=/data/index.bin

EXPOSE 8080

CMD ["/usr/local/bin/server"]