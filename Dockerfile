# syntax=docker/dockerfile:1
# Three build stages, one runtime image. Node and Rust never ship.
#
# The cache mounts are what make a rebuild seconds rather than minutes: the
# cargo registry and target dir survive between builds, so only changed crates
# recompile. A cache mount is gone by the next instruction, which is why the
# binary is copied out inside the same RUN.

FROM node:22-slim AS ui
WORKDIR /ui
COPY ui/package*.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY ui/ ./
RUN npm run build

FROM rust:slim-bookworm AS api
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release && cp target/release/quantrig /quantrig

FROM python:3.12-slim-bookworm
# bubblewrap is the sandbox; everything else is what the strategies import.
RUN apt-get update && apt-get install -y --no-install-recommends bubblewrap \
    && rm -rf /var/lib/apt/lists/*
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install "backtestingfx[report]" "lse-data[frames]" pandas pyarrow numpy

WORKDIR /app
COPY --from=api /quantrig /usr/local/bin/quantrig
COPY --from=ui /ui/dist ui/dist
COPY runner/ runner/
COPY fetcher/ fetcher/

ENV QUANTRIG_ROOT=/app \
    QUANTRIG_UI=/app/ui/dist \
    QUANTRIG_DATA=/data \
    QUANTRIG_PYTHON_PREFIX=/usr/local \
    QUANTRIG_ADDR=0.0.0.0:9000
EXPOSE 9000
CMD ["quantrig"]
