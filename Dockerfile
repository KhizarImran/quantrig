# syntax=docker/dockerfile:1
# Three build stages, one runtime image. Node and Rust never ship.

FROM node:22-slim AS ui
WORKDIR /ui
COPY ui/package*.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

FROM rust:slim-bookworm AS api
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

FROM python:3.12-slim-bookworm
# bubblewrap is the sandbox; everything else is what the strategies import.
RUN apt-get update && apt-get install -y --no-install-recommends bubblewrap \
    && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir "backtestingfx[report]" pandas pyarrow numpy

WORKDIR /app
COPY --from=api /src/target/release/quantrig /usr/local/bin/quantrig
COPY --from=ui /ui/dist ui/dist
COPY runner/ runner/
COPY examples/ examples/

ENV QUANTRIG_ROOT=/app \
    QUANTRIG_UI=/app/ui/dist \
    QUANTRIG_DATA=/data \
    QUANTRIG_PYTHON_PREFIX=/usr/local \
    QUANTRIG_ADDR=0.0.0.0:9000
EXPOSE 9000
CMD ["quantrig"]
