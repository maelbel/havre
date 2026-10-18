# Havre — one image, one process.
# Build from the project root:  docker build -f docker/Dockerfile -t havre .

# 1. Web UI → static files
FROM node:22-bookworm-slim AS web
WORKDIR /app/web
RUN npm install -g pnpm@10.28.0
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# 2. Rust server → one binary
FROM rust:1-bookworm AS server
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p havre-server

# 3. Small runtime image
FROM debian:bookworm-slim
# Claude Code, for the "Claude Code (your Claude plan)" AI provider: Node.js
# from the official image, then the CLI. Sign it in from Settings › AI; its
# login lives in /home/havre (a volume, see docker-compose.yml).
ARG CLAUDE_CODE_VERSION=2.1.300
COPY --from=node:22-bookworm-slim /usr/local/bin/node /usr/local/bin/node
COPY --from=node:22-bookworm-slim /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION} \
    && npm cache clean --force \
    && useradd --system --create-home --home-dir /home/havre havre \
    && mkdir -p /srv/havre/data && chown havre /srv/havre/data
WORKDIR /srv/havre
COPY --from=server /app/target/release/havre /usr/local/bin/havre
COPY --from=web /app/web/.output/public ./web
ENV HOME=/home/havre \
    HAVRE_BIND=0.0.0.0:8080 \
    HAVRE_WEB_DIR=/srv/havre/web \
    HAVRE_DATA_DIR=/srv/havre/data
# The local copy of your mailbox (SQLite). Keep it in a volume.
VOLUME /srv/havre/data
# Claude Code's sign-in.
VOLUME /home/havre
USER havre
EXPOSE 8080
CMD ["havre"]
