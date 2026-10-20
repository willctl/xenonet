# The image Anthropic's OSS Scanner audits XenoNet in (https://github.com/anthropics/oss-scanner).
#
# The audit runs offline, so this build installs Node and the Rust that rust-toolchain.toml pins,
# fetches every npm package and crate, and builds what threat_model.md (beside this file) says to
# exercise: the page, the xenonet binary (the service, the command and the MCP servers), the
# Node-API addon the JS tests load in place of core/, and the Rust tests. Electron isn't fetched:
# Desktop needs a display, and its own code is plain JS.

FROM node:24.22.0-bookworm-slim@sha256:a5f4366e834695d5f363b920c104953a535d3a9864d1bdf74566a0ebe01daee5 AS node

FROM rust:1.99.1-bookworm@sha256:e12566d5abc17a0da741628782cdb504b68b8961f5bfcf677936bf3e8b32de29
ENV DEBIAN_FRONTEND=noninteractive \
    ELECTRON_SKIP_BINARY_DOWNLOAD=1 \
    CARGO_TERM_COLOR=never \
    CARGO_PROFILE_DEV_DEBUG=line-tables-only
# git for the repo-status reader and the tests that make repos; tmux for the delivery route that
# types into a tmux pane.
RUN apt-get update && apt-get install -y --no-install-recommends git tmux \
    && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src

# The toolchain the checkout pins, with clippy and rustfmt; then every dependency.
RUN rustup toolchain install
RUN npm ci && cargo fetch --locked

# The page (dist/, which the service serves), the binary at target/debug/xenonet, the addon at
# crates/xenonet-node/xenonet.<platform>-<arch>.node, and the Rust test binaries.
RUN npm run build \
    && cargo build --workspace --locked \
    && npm run rust:build -- --debug \
    && cargo test --workspace --locked --no-run

# The cheap tests, so a broken image shows. A failure doesn't stop the build.
RUN cargo test --workspace --locked || echo "WARNING: cargo test reported failures"
RUN npm test || echo "WARNING: npm test reported failures"
