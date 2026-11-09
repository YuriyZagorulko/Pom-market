FROM rust:1-bookworm AS build
WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/src ./src
COPY backend/migrations ./migrations
COPY backend/tests ./tests
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/pom-api /usr/local/bin/pom-api
COPY --from=build /src/target/release/pom-worker /usr/local/bin/pom-worker
USER nobody
EXPOSE 4321
CMD ["pom-api"]
