# syntax=docker/dockerfile:1
#
# urlet container image.
#
# Stage 1 compiles a static-ish musl binary with the dependencies cached in a
# separate layer; stage 2 is a minimal alpine runtime that carries nothing but
# the binary, CA certificates and timezone data.

# ------------------------------------------------------------------ builder
FROM rust:1-alpine AS builder

RUN apk add --no-cache build-base

WORKDIR /src

# Dependency layer: build with stub sources so `cargo build` only compiles the
# dependency graph. Rebuilt only when Cargo.toml / Cargo.lock change.
#
# The root package version is pinned to a constant first. Every release bumps it,
# and the version of the crate being built is not a dependency, so leaving it in
# the cache key would recompile the whole graph (axum, sqlx, argon2, rustls...)
# on every single release for no reason. The real manifest is restored below.
#
# Every artifact of the stub crate must be removed afterwards: `COPY` preserves
# the source mtimes from the build context, so a stale `liburlet*.rlib` would
# look newer than the real sources and cargo would happily reuse it.
COPY Cargo.toml Cargo.lock ./
RUN sed -i 's/^version = ".*"/version = "0.0.0"/' Cargo.toml \
    && sed -i '/^name = "urlet"$/,/^version = / s/^version = ".*"/version = "0.0.0"/' Cargo.lock \
    && mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src \
              target/release/urlet \
              target/release/deps/urlet* \
              target/release/deps/liburlet* \
              target/release/.fingerprint/urlet-*

# Application layer. The manifest is copied again to undo the version pin above
# (and to pick up any dependency change); cargo then rebuilds only this crate.
COPY migrations ./migrations
COPY templates ./templates
COPY src ./src
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release --locked \
    && strip target/release/urlet \
    && adduser -D -H -u 10001 urlet \
    && install -o urlet -g urlet -m 0755 target/release/urlet /urlet

# ------------------------------------------------------------------ runtime
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata \
    && addgroup -S -g 10001 urlet \
    && adduser -S -u 10001 -G urlet -H -s /sbin/nologin urlet \
    && mkdir -p /data/themes \
    && chown -R urlet:urlet /data

COPY --from=builder /urlet /usr/local/bin/urlet

USER 10001:10001

ENV URLET_BIND=0.0.0.0:8080 \
    URLET_DATABASE_URL="sqlite:///data/urlet.db?mode=rwc" \
    URLET_DATA_DIR=/data \
    URLET_THEMES_DIR=/data/themes \
    URLET_LOG=info \
    URLET_LOG_FORMAT=compact

VOLUME ["/data"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/urlet", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/urlet"]
CMD ["serve"]
