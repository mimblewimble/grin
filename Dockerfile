# Multistage docker build, requires docker 17.05

# builder stage
FROM rust:1.26 as builder

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

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y locales

RUN sed -i -e 's/# en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen && \
    dpkg-reconfigure --frontend=noninteractive locales && \
    update-locale LANG=en_US.UTF-8

ENV LANG en_US.UTF-8

COPY --from=builder /usr/src/grin/target/release/grin /usr/local/bin/grin
COPY --from=builder /usr/src/grin/grin.toml /usr/src/grin/grin.toml

WORKDIR /usr/src/grin

EXPOSE 13413
EXPOSE 13414
EXPOSE 13415
EXPOSE 13416

ENTRYPOINT ["grin", "server", "run"]
