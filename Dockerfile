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

# Create grin user
RUN useradd -ms /bin/bash grin
USER grin

RUN mkdir ~/.grin
VOLUME ["/home/grin/.grin"]

# Create mainnet config
WORKDIR /home/grin/.grin/main
RUN grin server config
RUN sed -i '/^run_tui /s/=.*$/= true/' grin-server.toml

# Create testnet config
WORKDIR /home/grin/.grin/test
RUN grin --testnet server config
RUN sed -i '/^run_tui /s/=.*$/= true/' grin-server.toml

# Mainnet ports
EXPOSE 3413 3414

# Testnet ports
EXPOSE 13413 13414

# Stratum port
EXPOSE 3416

ENTRYPOINT ["grin", "server", "run"]