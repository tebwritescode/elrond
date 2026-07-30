# syntax=docker/dockerfile:1.7

FROM node:24-alpine AS web-builder
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.97-bookworm AS rust-builder
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ ./crates/
COPY migrations/ ./migrations/
RUN cargo build --locked --release -p elrond-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 elrond \
    && useradd --system --uid 10001 --gid elrond --home-dir /app --shell /usr/sbin/nologin elrond \
    && mkdir --parents /app/web /data \
    && chown --recursive elrond:elrond /app /data

COPY --from=rust-builder /build/target/release/elrond-server /usr/local/bin/elrond
COPY --from=web-builder /build/web/dist/ /app/web/

ENV ELROND_BIND_ADDRESS=0.0.0.0:3000 \
    ELROND_DATABASE_URL=sqlite:///data/elrond.db?mode=rwc \
    ELROND_DATA_DIR=/data \
    ELROND_WEB_DIR=/app/web \
    ELROND_SECURE_COOKIES=true \
    RUST_LOG=elrond=info,tower_http=info

USER elrond
WORKDIR /app
VOLUME ["/data"]
EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:3000/api/health || exit 1

ENTRYPOINT ["/usr/local/bin/elrond"]
