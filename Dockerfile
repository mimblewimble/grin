# Builder
FROM rust:slim-trixie AS builder

WORKDIR /usr/src/grin
COPY . .

RUN apt update && \
    apt install -y libncurses5-dev libncursesw5-dev

RUN cargo build --release

# Runner
FROM debian:trixie-slim
COPY --from=builder /usr/src/grin/target/release/grin /usr/local/bin/grin

RUN apt update && \
    apt install -y libncursesw5-dev

# Create mainnet config
WORKDIR /root/.grin/main
RUN grin server config
RUN sed -i '/^run_tui /s/=.*$/= false/' grin-server.toml

# Create testnet config
WORKDIR /root/.grin/test
RUN grin --testnet server config
RUN sed -i '/^run_tui /s/=.*$/= false/' grin-server.toml

# Mainnet ports
EXPOSE 3413 3414

# Testnet ports
EXPOSE 13413 13414

# Stratum port
EXPOSE 3416

WORKDIR /root/.grin
ENTRYPOINT ["grin"]
CMD ["server", "run"]