# The web UI, which the server embeds at compile time.
FROM node:24-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# The server binary.
FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY migrations/ migrations/
COPY --from=web /src/web/dist/ web/dist/
RUN cargo build --release --locked -p lfm-mirror \
    && cp target/release/lfm-mirror /usr/local/bin/lfm-mirror

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data lfm \
    && mkdir /data && chown lfm /data
COPY --from=server /usr/local/bin/lfm-mirror /usr/local/bin/lfm-mirror
USER lfm
WORKDIR /data
ENV LFM_DATA_DIR=/data \
    LFM_BIND=0.0.0.0:3055 \
    RUST_LOG=info
VOLUME /data
EXPOSE 3055
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s \
    CMD curl -fsS http://127.0.0.1:3055/healthz || exit 1
ENTRYPOINT ["/usr/local/bin/lfm-mirror"]
