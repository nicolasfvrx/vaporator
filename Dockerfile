# syntax=docker/dockerfile:1
FROM rust:1.88-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY locales ./locales
COPY migrations ./migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked && cp target/release/vaporator /build/vaporator

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home vaporator \
    && install -d -m 0700 -o vaporator -g vaporator /data
COPY --from=builder /build/vaporator /usr/local/bin/vaporator
USER vaporator
ENV DATA_DIR=/data
WORKDIR /data
ENTRYPOINT ["vaporator"]
CMD ["run"]
