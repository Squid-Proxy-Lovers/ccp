FROM rust:1-bookworm AS builder

WORKDIR /app

COPY Cargo.toml /app/Cargo.toml
COPY Cargo.lock /app/Cargo.lock

# Copy all workspace member manifests so cargo can resolve the workspace.
# Only protocol and server get full source trees — client and tests get
# dummy lib.rs files since we don't need their binaries in the image.
COPY crates/protocol/Cargo.toml /app/crates/protocol/Cargo.toml
COPY crates/protocol/src /app/crates/protocol/src
COPY crates/server/Cargo.toml /app/crates/server/Cargo.toml
COPY crates/server/src /app/crates/server/src
COPY crates/client/Cargo.toml /app/crates/client/Cargo.toml
RUN mkdir -p /app/crates/client/src && echo "" > /app/crates/client/src/lib.rs && echo "fn main(){}" > /app/crates/client/src/main.rs
COPY tests/Cargo.toml /app/tests/Cargo.toml
RUN mkdir -p /app/tests/src && echo "" > /app/tests/src/lib.rs

RUN cargo build --release -p server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system ccp \
    && useradd --system --gid ccp --create-home --home-dir /var/lib/ccp ccp \
    && mkdir -p /var/lib/ccp/server \
    && chown -R ccp:ccp /var/lib/ccp

COPY --from=builder /app/target/release/server /usr/local/bin/server
COPY docker/server-entrypoint.sh /usr/local/bin/ccp-server-entrypoint

RUN chmod +x /usr/local/bin/ccp-server-entrypoint

ENV CCP_SERVER_DATA_DIR=/var/lib/ccp/server

USER ccp
WORKDIR /var/lib/ccp

VOLUME ["/var/lib/ccp/server"]
EXPOSE 1337 1338

ENTRYPOINT ["/usr/local/bin/ccp-server-entrypoint"]
