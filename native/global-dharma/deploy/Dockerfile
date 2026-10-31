FROM rust:1.85-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p global-dharma-daemon
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/* && useradd --system --home /var/lib/global-dharma --shell /usr/sbin/nologin global-dharma
COPY --from=build /src/target/release/global-dharma-daemon /usr/local/bin/global-dharma-daemon
USER global-dharma
EXPOSE 18888
ENTRYPOINT ["/usr/local/bin/global-dharma-daemon"]
