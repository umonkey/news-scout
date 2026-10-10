# Build stage
# Pin the toolchain so base-image drift does not invalidate the cached
# dependency layer on every build; keep in sync with check-news-scout.yml.
FROM rust:1.99.0 AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libsqlite3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Dummy build to cache dependencies between builds.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --locked
RUN rm -rf src

# Copy and build the actual source code.
COPY . .
RUN touch src/main.rs
RUN cargo build --release --locked && \
    cp target/release/news-scout /news-scout

# Final stage
FROM debian:stable-slim

# Set environment variables
ENV TZ=Asia/Yerevan

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libsqlite3-0 \
    sqlite3 \
    ca-certificates \
    tzdata \
    && ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user
RUN groupadd -r scout && useradd -r -g scout -s /bin/bash scout

WORKDIR /app

# Copy the binary from the builder stage
COPY --from=builder /news-scout /app/news-scout
COPY analyze-prompt.md /app/analyze-prompt.md

# Ensure the app directory is owned by the scout user
RUN mkdir -p /app/var && chown -R scout:scout /app

USER scout

# Start both the reader and the sender as supervised tasks in one process
CMD ["./news-scout", "run"]
