# Multistage docker build, requires docker 17.05

# builder stage
FROM rust:1.29.0 as builder

RUN set -ex && \
    apt-get update && \
    apt-get --no-install-recommends --yes install \
        clang \
        libclang-dev \
        llvm-dev \
        libncurses5 \
        libncursesw5 \
        cmake \
        git

WORKDIR /usr/src/grin

# Copying Grin
COPY . .

# Building Grin
RUN cargo build --release

# runtime stage
FROM debian:9.4

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y locales openssl

RUN sed -i -e 's/# en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen && \
    dpkg-reconfigure --frontend=noninteractive locales && \
    update-locale LANG=en_US.UTF-8

ENV LANG en_US.UTF-8

COPY --from=builder /usr/src/grin/target/release/grin /usr/local/bin/grin

WORKDIR /root/.grin
RUN grin server config && \
    sed -i -e 's/run_tui = true/run_tui = false/' grin-server.toml

EXPOSE 13413 13414 13415 13416

ENTRYPOINT ["grin", "server", "run"]
