FROM rust:1-bookworm AS builder
WORKDIR /app
# Build the query macros from the committed .sqlx metadata, not a live database.
ENV SQLX_OFFLINE=true
COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home --shell /usr/sbin/nologin blackwall
COPY --from=builder /app/target/release/blackwall-backend /usr/local/bin/blackwall-backend
USER blackwall
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/blackwall-backend"]
