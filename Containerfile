# git-cdc-server container image
#   podman build -f Containerfile -t git-cdc-server .
#   docker run -p 8077:8077 -v cdc-data:/data -e GIT_CDC_TOKEN=secret ghcr.io/amjadjibon/git-cdc-server

# Builder and runtime must share a Debian release (glibc compatibility).
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p git-cdc-server

FROM debian:bookworm-slim
# ca-certificates: TLS to real AWS S3 endpoints
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir /data && chown 65534:65534 /data
COPY --from=build /src/target/release/git-cdc-server /usr/local/bin/git-cdc-server

USER 65534:65534
# The binary's default listen is 127.0.0.1 — unreachable from outside a
# container; bind all interfaces here instead.
ENV GIT_CDC_LISTEN=0.0.0.0:8077
ENV GIT_CDC_ROOT=/data
EXPOSE 8077
VOLUME /data
ENTRYPOINT ["git-cdc-server"]
