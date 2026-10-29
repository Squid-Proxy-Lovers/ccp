FROM node:24-bookworm-slim AS panel
WORKDIR /panel
COPY crates/server/web/package.json crates/server/web/package-lock.json ./
RUN npm ci --ignore-scripts
COPY crates/server/web/src ./src
COPY crates/server/web/public ./public
COPY crates/server/web/index.html crates/server/web/vite.config.ts crates/server/web/tsconfig.json ./
RUN npm run check && npm run build

FROM rust:1.99.0-bookworm AS builder

WORKDIR /app

COPY Cargo.toml /app/Cargo.toml
COPY Cargo.lock /app/Cargo.lock
COPY rust-toolchain.toml /app/rust-toolchain.toml
COPY .cargo/config.toml /app/.cargo/config.toml

# Copy all workspace member manifests so cargo can resolve the workspace.
# Protocol, transport and server get full source trees — client and tests get
# dummy lib.rs files since we don't need their binaries in the image.
COPY crates/protocol/Cargo.toml /app/crates/protocol/Cargo.toml
COPY crates/protocol/src /app/crates/protocol/src
COPY crates/transport/Cargo.toml /app/crates/transport/Cargo.toml
COPY crates/transport/src /app/crates/transport/src
COPY crates/server/Cargo.toml /app/crates/server/Cargo.toml
COPY crates/server/src /app/crates/server/src
COPY crates/server/build.rs /app/crates/server/build.rs
COPY --from=panel /panel/dist /app/panel-dist
ENV CCP_PANEL_DIST=/app/panel-dist
COPY crates/client/Cargo.toml /app/crates/client/Cargo.toml
RUN mkdir -p /app/crates/client/src && echo "" > /app/crates/client/src/lib.rs && echo "fn main(){}" > /app/crates/client/src/main.rs
COPY tests/Cargo.toml /app/tests/Cargo.toml
RUN mkdir -p /app/tests/src && echo "" > /app/tests/src/lib.rs

RUN cargo build --release --locked -p server

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

RUN chmod +x /usr/local/bin/ccp-server-entrypoint \
    && ln -s /usr/local/bin/server /usr/local/bin/ccp-server

ENV CCP_SERVER_DATA_DIR=/var/lib/ccp/server

USER ccp
WORKDIR /var/lib/ccp

VOLUME ["/var/lib/ccp/server"]
EXPOSE 1337 1338 8443 8444

ENTRYPOINT ["/usr/local/bin/ccp-server-entrypoint"]
