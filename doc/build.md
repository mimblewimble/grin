# Grin - Build, Configuration, and Running

*Read this in other languages: [Español](translations/build_ES.md), [Korean](translations/build_KR.md), [日本語](translations/build_JP.md), [简体中文](translations/build_ZH-CN.md).*

## Overview

This document explains how to build Grin from source. It covers every supported operating system and walks you through installing the required tools, building the binary, and running a Grin node.

The general build flow is the same on all platforms:

1. Install system dependencies.
2. Install Rust via [rustup](https://rustup.rs).
3. Clone the repository.
4. Build with Cargo.
5. Run the resulting binary.

---

## Supported Platforms

The following platforms are actively tested in CI and have release binaries produced for every tagged version:

| Platform | Architecture | Status |
|---|---|---|
| Linux | x86\_64 | Fully supported |
| macOS | x86\_64 (Intel) | Fully supported |
| macOS | arm64 (Apple Silicon) | Fully supported |
| Windows | x86\_64 | Fully supported |

Cross-compilation to other targets (e.g. ARM Linux for a Raspberry Pi) is possible through Cargo, but is not officially tested in CI.

---

## Rust Toolchain

Grin uses the **Rust 2021 edition** and requires a recent stable toolchain. No `rust-toolchain` file is present in the repository, so the latest stable release of Rust is recommended.

Install or update Rust using [rustup](https://rustup.rs):

```sh
# Install rustup (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Make the Rust toolchain available in the current shell
source "$HOME/.cargo/env"
```

If you already have Rust installed, update to the latest stable version:

```sh
rustup update
```

Verify your installation:

```sh
rustc --version
cargo --version
```

---

## Linux

### Prerequisites

Install the required system libraries. On **Debian/Ubuntu/Mint** and other `apt`-based distributions, run:

```sh
sudo apt update
sudo apt install \
    build-essential \
    cmake \
    git \
    clang \
    libgit2-dev \
    libncurses5-dev \
    libncursesw5-dev \
    libssl-dev \
    llvm \
    pkg-config \
    zlib1g-dev
```

On **Fedora/RHEL/CentOS** and other `dnf`/`yum`-based distributions:

```sh
sudo dnf install \
    cmake \
    git \
    clang \
    libgit2-devel \
    ncurses-devel \
    openssl-devel \
    llvm \
    pkgconf \
    zlib-devel
```

On **Alpine Linux**, also install `linux-headers`:

```sh
sudo apk add \
    build-base \
    cmake \
    git \
    clang \
    libgit2-dev \
    ncurses-dev \
    openssl-dev \
    llvm \
    pkgconf \
    linux-headers \
    zlib-dev
```

Then install Rust as described in the [Rust Toolchain](#rust-toolchain) section above.

### Build from Source

```sh
# 1. Clone the repository
git clone https://github.com/mimblewimble/grin.git
cd grin

# 2. Build a release binary (recommended)
cargo build --release
```

The `--release` flag enables full compiler optimisations. Omitting it produces a debug build which is significantly slower for cryptographic operations and will make fast sync prohibitively slow.

### Run

Add the release binary to your `PATH` for convenience:

```sh
export PATH="$PWD/target/release:$PATH"
```

Then start a Grin node:

```sh
grin server run
```

The binary is also available directly at `target/release/grin` without modifying `PATH`.

---

## macOS

### Prerequisites

1. Install the Xcode Command Line Tools:

    ```sh
    xcode-select --install
    ```

2. Install [Homebrew](https://brew.sh) if you do not already have it:

    ```sh
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    ```

3. Install the required packages:

    ```sh
    brew install llvm pkg-config openssl
    ```

4. Install Rust as described in the [Rust Toolchain](#rust-toolchain) section above.

### Build from Source

```sh
# 1. Clone the repository
git clone https://github.com/mimblewimble/grin.git
cd grin

# 2. Build a release binary (recommended)
cargo build --release
```

#### Apple Silicon (arm64)

No additional steps are needed on Apple Silicon Macs. `cargo build --release` produces a native arm64 binary.

#### Intel (x86\_64)

On Intel Macs the default build already targets `x86_64-apple-darwin`. If you need to produce an Intel binary from an Apple Silicon Mac explicitly, first add the target:

```sh
rustup target add x86_64-apple-darwin
cargo build --release --target x86_64-apple-darwin
```

The binary will then be at `target/x86_64-apple-darwin/release/grin`.

### Run

Add the release binary to your `PATH` for convenience:

```sh
export PATH="$PWD/target/release:$PATH"
```

Then start a Grin node:

```sh
grin server run
```

---

## Windows

Windows builds are fully supported and tested in CI. The MSVC toolchain is used and the binary is statically linked against the CRT (configured in `.cargo/config.toml`).

### Prerequisites

1. Install [Rust via rustup](https://rustup.rs). The installer will prompt you to install the **MSVC** toolchain; choose the default options.

2. Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (or a full Visual Studio installation) and select the **"Desktop development with C++"** workload. This provides the MSVC compiler and linker that Cargo requires.

3. Install [Git for Windows](https://git-scm.com/download/win).

> **Note:** The LLVM/Clang toolchain is not required on Windows. The build uses the MSVC compiler.

### Build from Source

Open a **Developer Command Prompt** or a regular PowerShell/Command Prompt (after Rust is on your `PATH`) and run:

```powershell
# 1. Clone the repository
git clone https://github.com/mimblewimble/grin.git
cd grin

# 2. Set the required environment variable for the Roaring bitmap crate
$env:ROARING_ARCH = "x86-64-v2"

# 3. Build a release binary (recommended)
cargo build --release
```

### Run

```powershell
.\target\release\grin.exe server run
```

Or add `target\release` to your `PATH` and run `grin server run`.

---

## Build Modes

| Mode | Command | Use case |
|---|---|---|
| Release (recommended) | `cargo build --release` | Running a node; fast sync |
| Debug | `cargo build` | Development and quick iteration |

> **Important:** Debug builds disable compiler optimisations. Cryptographic operations run significantly slower, which makes fast sync impractical. Always use a release build when syncing the full chain.

---

## What Was Built?

A successful build produces:

| Platform | Binary path |
|---|---|
| Linux / macOS | `target/release/grin` |
| macOS (cross-compiled Intel) | `target/x86_64-apple-darwin/release/grin` |
| Windows | `target\release\grin.exe` |

---

## Running Grin

### First Run

On the first run, Grin creates its default configuration directory at `~/.grin/main/` (Linux/macOS) or `%USERPROFILE%\.grin\main\` (Windows) and writes a `grin-server.toml` file there.

### Running in the Current Directory

To keep all data files in the current working directory instead of the home directory, generate a local configuration file first:

```sh
grin server config
```

This writes a `grin-server.toml` file in the current directory. Subsequent runs of `grin` from the same directory will use this file automatically.

### Useful Commands

```sh
grin help
grin server --help
grin client --help
```

---

## Verifying the Build

Run the test suite to confirm the build is working correctly. Tests are run per-crate.

Run all tests at once:

```sh
cargo test --release --all
```

Or test individual components:

```sh
cargo test --release -p grin_chain
cargo test --release -p grin_core
cargo test --release -p grin_keychain
cargo test --release -p grin_pool
cargo test --release -p grin_p2p
cargo test --release -p grin_servers
cargo test --release -p grin_api
cargo test --release -p grin_util
cargo test --release -p grin_store
```

The test suite is the same suite run in CI for every pull request.

---

## Configuration

Grin runs with sensible defaults out of the box and can be further tuned via `grin-server.toml`. The file is auto-generated on first run and contains inline documentation for every option.

Command-line switches always override values in `grin-server.toml`.

---

## Docker

A `Dockerfile` is included in the repository root. Build and run a containerised Grin node with:

```sh
# Build the image
docker build -t grin .

# Run, mounting the host ~/.grin directory into the container
docker run -it -d -v "$HOME/.grin:/root/.grin" grin
```

To use a named Docker volume instead:

```sh
docker run -it -d -v dotgrin:/root/.grin grin
```

---

## Common Build Problems

### `error: linker 'cc' not found` (Linux)

The C linker is not installed. Install `build-essential` (Debian/Ubuntu) or `gcc` (Fedora/Alpine) and try again.

### `error[E0463]: can't find crate for 'std'` (Windows, wrong toolchain)

Make sure the **MSVC** Rust target is installed, not GNU:

```powershell
rustup target add x86_64-pc-windows-msvc
rustup default stable-x86_64-pc-windows-msvc
```

### OpenSSL not found (Linux/macOS)

Ensure `pkg-config` and `libssl-dev` (Linux) or `openssl` (macOS via Homebrew) are installed. On macOS you may also need to set:

```sh
export OPENSSL_DIR=$(brew --prefix openssl)
```

### LLVM/Clang not found (Linux/macOS)

Install `llvm` and `clang` via your package manager. On macOS, `brew install llvm` and ensure the Homebrew LLVM `bin` directory is on your `PATH`:

```sh
export PATH="$(brew --prefix llvm)/bin:$PATH"
```

### Build is slow / fast sync is unusable

This is expected behaviour for debug builds. Rebuild with `--release`:

```sh
cargo build --release
```

### More Help

See the [Troubleshooting wiki page](https://github.com/mimblewimble/docs/wiki/Troubleshooting) for additional issues.

---

## Mining

All mining functionality has moved to a separate package, [grin-miner](https://github.com/mimblewimble/grin-miner). To mine:

1. Run a Grin node with Stratum enabled in `grin-server.toml`:

    ```toml
    enable_stratum_server = true
    ```

2. Start a wallet listener:

    ```sh
    grin-wallet listen
    ```

3. Build and run grin-miner against your running node.

---

## Using Grin

The wiki page [Wallet User Guide](https://github.com/mimblewimble/docs/wiki/Wallet-User-Guide) and linked pages have more information on features and troubleshooting.
