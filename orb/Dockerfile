FROM rust:1-slim-trixie@sha256:4df2ae8cfdfebdfc13a65da7f253b73d68983dc0339f2719bc17a279c913b13d AS builder
ARG CRATE_VERSION
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    cmake \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && { err=/tmp/cargo-install.err; n=0; \
       until cargo install jci-coverage --locked \
             --version "${CRATE_VERSION}" 2>"$err"; do \
         cat "$err" >&2; \
         grep -q "failed to compile" "$err" \
           && echo "build failed, not an index delay" >&2 && exit 1; \
         n=$((n+1)); \
         [ "$n" -ge 40 ] \
           && echo "crates.io index never served ${CRATE_VERSION}" >&2 \
           && exit 1; \
         echo "waiting for crates.io index: ${CRATE_VERSION} (try $n)"; \
         rm -rf "${CARGO_HOME:-/usr/local/cargo}"/registry/index/*/.cache; \
         sleep 15; \
       done; }
RUN cargo install cargo-binstall --locked \
    && cargo binstall --no-confirm \
    cargo-llvm-cov@0.9.1 \
    cargo-nextest@0.9.146

FROM rust:1-slim-trixie@sha256:4df2ae8cfdfebdfc13a65da7f253b73d68983dc0339f2719bc17a279c913b13d
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    gnupg \
    openssh-client \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -ms /bin/bash circleci
COPY --from=builder /usr/local/cargo/bin/jci-coverage /usr/local/bin/jci-coverage
COPY --from=builder /usr/local/cargo/bin/cargo-llvm-cov /usr/local/bin/cargo-llvm-cov
COPY --from=builder /usr/local/cargo/bin/cargo-nextest /usr/local/bin/cargo-nextest
RUN rustup component add \
    llvm-tools-preview
USER circleci
WORKDIR /home/circleci/project
