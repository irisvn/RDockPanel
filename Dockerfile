# RDockPanel's image: the panel's one binary (its admin pages are built in),
# its configuration, and an entrypoint that migrates before serving.
#   docker build -t rdockpanel .
# The installer (install/install.sh) runs it as a Swarm service.
# The dependencies are built in a layer of their own (cargo-chef), cached
# by CI between builds: a new commit rebuilds the panel's crate only.
FROM rust:1-bookworm AS chef
RUN cargo install cargo-chef --locked --version ^0.1
WORKDIR /src

FROM chef AS plan
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=plan /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
# What the binary says of itself (build.rs): the release, the commit
# (declared after the dependencies: a new commit keeps their layer).
ARG RDP_VERSION=""
ARG RDP_COMMIT=""
ENV RDP_VERSION=${RDP_VERSION} RDP_COMMIT=${RDP_COMMIT}
COPY . .
RUN cargo build --release --locked \
 && cp target/release/rdockpanel /rdockpanel \
 && strip /rdockpanel

FROM debian:bookworm-slim
ARG TARGETARCH
# Git and SSH clone the services' repositories (and make deploy keys);
# Docker's CLI runs Compose services.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini curl git openssh-client \
 && rm -rf /var/lib/apt/lists/* \
 && install -m 0755 -d /etc/apt/keyrings \
 && curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc \
 && echo "deb [arch=${TARGETARCH:-amd64} signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian bookworm stable" > /etc/apt/sources.list.d/docker.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends docker-ce-cli docker-compose-plugin \
 && rm -rf /var/lib/apt/lists/*
# Declared after the installs: a new commit keeps their cached layers.
ARG RDP_VERSION=""
ARG RDP_COMMIT=""
LABEL org.opencontainers.image.title="RDockPanel" \
      org.opencontainers.image.source="https://github.com/irisvn/rdockpanel" \
      org.opencontainers.image.version="${RDP_VERSION}" \
      org.opencontainers.image.revision="${RDP_COMMIT}"
WORKDIR /app
COPY --from=build /rdockpanel /app/rdockpanel
COPY config /app/config
COPY install/entrypoint.sh /app/entrypoint.sh
RUN mkdir -p /app/storage && chmod 0755 /app/entrypoint.sh
ENV APP_ENV=production \
    APP_DEBUG=false \
    RUSTCMS_APP__LISTEN=0.0.0.0:3000 \
    RUSTCMS_PANEL__TRAEFIK_DIR=/etc/rdockpanel/traefik/dynamic
EXPOSE 3000
ENTRYPOINT ["/usr/bin/tini", "--", "/app/entrypoint.sh"]
CMD ["serve"]
