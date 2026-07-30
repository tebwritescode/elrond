# Elrond: one image serving the API, the built client, and background work.
#
# Multi-stage so the runtime layer carries no compiler, no Node, and no source.

# ---------------------------------------------------------------- client build
FROM node:24-alpine AS web-build
WORKDIR /build/web

# Manifests first, so the dependency layer is cached until they actually change.
COPY web/package.json web/package-lock.json ./
RUN npm ci

COPY web/ ./
# `npm run build` typechecks before bundling, so a type error fails the image
# build rather than shipping.
RUN npm run build

# ------------------------------------------------------------------ api build
FROM rust:1-alpine AS api-build
WORKDIR /build

# musl-dev and the SQLite headers are needed for a static link; `mold` is not
# used because the marginal gain is not worth another moving part.
RUN apk add --no-cache musl-dev pkgconf

# Manifests and empty sources first, so the dependency compile is cached
# independently of the application code.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/domain/Cargo.toml crates/domain/
COPY crates/application/Cargo.toml crates/application/
COPY crates/infrastructure/Cargo.toml crates/infrastructure/
COPY crates/api/Cargo.toml crates/api/
COPY crates/server/Cargo.toml crates/server/
RUN mkdir -p crates/domain/src crates/application/src crates/infrastructure/src \
             crates/api/src crates/server/src \
 && echo '' > crates/domain/src/lib.rs \
 && echo '' > crates/application/src/lib.rs \
 && echo '' > crates/infrastructure/src/lib.rs \
 && echo '' > crates/api/src/lib.rs \
 && echo 'fn main() {}' > crates/server/src/main.rs \
 && cargo build --release --locked \
 && rm -rf crates/*/src

COPY crates/ crates/
COPY migrations/ migrations/
# Touch every entry point so cargo does not reuse the placeholder artifacts.
RUN find crates -name '*.rs' -exec touch {} + \
 && cargo build --release --locked -p elrond-server

# -------------------------------------------------------------------- runtime
FROM alpine:3.22 AS runtime

# ca-certificates for outbound TLS to Stirling-PDF; tzdata so timestamps render
# correctly if an operator sets TZ.
RUN apk add --no-cache ca-certificates tzdata \
 && addgroup -g 10001 -S elrond \
 && adduser -u 10001 -S -G elrond -h /app elrond

WORKDIR /app
COPY --from=api-build /build/target/release/elrond /usr/local/bin/elrond
COPY --from=web-build /build/web/dist /app/web

# The volume is created owned by the runtime user so the process never needs to
# start as root to fix permissions.
RUN mkdir -p /data && chown -R elrond:elrond /data /app

USER elrond:elrond

ENV ELROND_BIND_ADDRESS=0.0.0.0:3100 \
    ELROND_DATA_DIR=/data \
    ELROND_DATABASE_URL="sqlite:///data/elrond.db?mode=rwc" \
    ELROND_WEB_DIR=/app/web \
    RUST_LOG=info

# Documents, the database, and generated binders all live here.
VOLUME ["/data"]

EXPOSE 3100

# Uses the API's own health endpoint rather than a TCP probe, so a process that
# is listening but cannot reach its database is still reported unhealthy.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD wget --quiet --tries=1 --spider http://127.0.0.1:3100/api/v1/health || exit 1

ENTRYPOINT ["/usr/local/bin/elrond"]
