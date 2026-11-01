# Apiary — one container, one machine, one volume.
#
#   docker build -t apiary .
#   docker run --rm -p 3000:3000 -v apiary-data:/data \
#     -e AUTUMN_SECURITY__SIGNING_SECRET="$(openssl rand -hex 32)" \
#     -e AUTUMN_SECURITY__TRUSTED_HOSTS__HOSTS=localhost apiary

# ── build ────────────────────────────────────────────────────────────────────
# Matches rust-toolchain.toml.
FROM rust:1.97.1-slim-bookworm AS builder
WORKDIR /src

# Cache dependencies: build once against stub sources, then the real ones.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/hive/Cargo.toml crates/hive/Cargo.toml
COPY crates/apiary-app/Cargo.toml crates/apiary-app/Cargo.toml
RUN mkdir -p crates/hive/src crates/apiary-app/src \
    && echo '' > crates/hive/src/lib.rs \
    && echo '' > crates/apiary-app/src/lib.rs \
    && echo 'fn main() {}' > crates/apiary-app/src/main.rs \
    && cargo build --release -p apiary-app \
    && rm -rf crates/*/src

COPY crates crates
# Touch so cargo sees the real sources as newer than the stub build.
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release -p apiary-app

# ── runtime ──────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /app apiary \
    && mkdir -p /app /data && chown apiary:apiary /app /data

WORKDIR /app
COPY --from=builder /src/target/release/apiary-app /usr/local/bin/apiary-app
COPY autumn.toml autumn-prod.toml ./
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

# Every piece of state is a file under /data (mount a volume there).
ENV AUTUMN_PROFILE=prod \
    AUTUMN_DATABASE__URL=sqlite:///data/apiary-control.db \
    AUTUMN_DATABASE__FLEET__ROOT=/data/apiary-fleet \
    APIARY_ARCHIVE_DIR=/data/apiary-archive \
    APIARY_HARVEST_DB=/data/apiary-harvest.db

USER apiary
EXPOSE 3000
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD ["/usr/local/bin/apiary-app"]
