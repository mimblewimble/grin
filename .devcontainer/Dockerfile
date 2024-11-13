# Rust latest
FROM mcr.microsoft.com/devcontainers/rust:latest

# Install Required Dependencies
RUN apt-get -qq update
RUN apt-get install -y build-essential cmake git libgit2-dev clang libncurses5-dev libncursesw5-dev zlib1g-dev pkg-config libssl-dev llvm

# Install snapd and its dependencies
RUN apt-get install -y snapd fuse squashfuse sudo gh

# Create a script to start snapd
RUN echo '#!/bin/bash\n\
/usr/lib/snapd/snapd &\n\
sleep 5\n\
exec "$@"' > /start-snapd.sh && \
    chmod +x /start-snapd.sh


VOLUME ["/sys/fs/cgroup"]

# Use the script as the entry point
ENTRYPOINT ["/start-snapd.sh"]
CMD ["bash"]