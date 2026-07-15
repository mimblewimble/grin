# Rust latest
FROM mcr.microsoft.com/devcontainers/rust:latest

# Install Required Dependencies
RUN apt-get -qq update
RUN apt-get install -y \
    build-essential \
    clang \
    cmake \
    fuse \
    gh \
    git \
    libgit2-dev \
    libncurses5-dev \
    libncursesw5-dev \
    libssl-dev \
    llvm \
    pkg-config \
    snapd \
    squashfuse \
    sudo \
    zlib1g-dev

RUN echo '#!/bin/bash\n\
/usr/lib/snapd/snapd &\n\
sleep 5\n\
exec "$@"' > /start-snapd.sh && \
    chmod +x /start-snapd.sh

VOLUME ["/sys/fs/cgroup"]

ENTRYPOINT ["/start-snapd.sh"]
CMD ["bash"]
