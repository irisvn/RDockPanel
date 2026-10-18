# RDockPanel's image: the panel's one binary (its admin pages are built in),
# its configuration, and an entrypoint that migrates before serving.
#   docker build -t rdockpanel .
# The installer (install/install.sh) runs it as a Swarm service.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && cp target/release/rdockpanel /rdockpanel \
 && strip /rdockpanel

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini curl \
 && rm -rf /var/lib/apt/lists/*
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
