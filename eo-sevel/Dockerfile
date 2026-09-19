FROM rust:1.97-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY migrations ./migrations
COPY resources ./resources
RUN cargo build --locked --release

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/eo-sevel /app/eo-sevel
COPY static /app/static
ENV ELEVAGE_DATA=/data EO_HOST=0.0.0.0 EO_PORT=8080
VOLUME ["/data"]
EXPOSE 8080
CMD ["/app/eo-sevel"]
