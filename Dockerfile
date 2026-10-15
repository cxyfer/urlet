# syntax=docker/dockerfile:1
#
# uwurls container image.
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
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/uwurls target/release/deps/uwurls*

# Application layer.
COPY migrations ./migrations
COPY templates ./templates
COPY src ./src
RUN cargo build --release --locked \
    && strip target/release/uwurls \
    && adduser -D -H -u 10001 uwurls \
    && install -o uwurls -g uwurls -m 0755 target/release/uwurls /uwurls

# ------------------------------------------------------------------ runtime
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata \
    && addgroup -S -g 10001 uwurls \
    && adduser -S -u 10001 -G uwurls -H -s /sbin/nologin uwurls \
    && mkdir -p /data \
    && chown -R uwurls:uwurls /data

COPY --from=builder /uwurls /usr/local/bin/uwurls

USER 10001:10001

ENV UWURLS_BIND=0.0.0.0:8080 \
    UWURLS_DATABASE_URL="sqlite:///data/uwurls.db?mode=rwc" \
    UWURLS_DATA_DIR=/data \
    UWURLS_LOG=info \
    UWURLS_LOG_FORMAT=compact

VOLUME ["/data"]
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/uwurls", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/uwurls"]
CMD ["serve"]
