# Grin - Build, Configuration, and Running

*다른 언어로 되어있는 문서를 읽으려면: [English](../build.md), [Español](build_ES.md), [日本語](build_JP.md), [简体中文](build_ZH-CN.md).*

## 지원하는 플랫폼들에 대해서

장기적으로는 대부분의 플랫폼에서 어느정도 지원하게 될 것입니다.
Grin 프로그래밍 언어는 `rust`로 대부분의 플랫폼들에서 빌드 할 수 있습니다.

지금까지 작동하는 플랫폼은 무엇인가요?

* Linux x86_64 그리고 macOS [grin + mining + development]
* Windows 10은 아직 지원하지 않습니다 [grin kind-of builds, mining은 아직 지원하지 않음 . 도움이 필요해요!]

## 요구사항

* rust 1.34 버전 이상  (다음 명령어를 사용하세요. [rustup]((https://www.rustup.rs/))- 예.) `curl https://sh.rustup.rs -sSf | sh; source $HOME/.cargo/env`)

  * 만약 rust 가 설치되어 있다면, 다음 명령어를 사용해서 업데이트 할 수 있습니다.
    `rustup update`
* clang
* ncurses 과 libs (ncurses, ncursesw5)
* zlib libs (zlib1g-dev or zlib-devel)
* pkg-config
* libssl-dev
* linux-headers (reported needed on Alpine linux)
* llvm

Debian 기반의 배포들은 (Debian, Ubuntu, Mint, 등등) 다음 명령어 한 줄로 설치 됩니다.

```sh
apt install build-essential cmake git libgit2-dev clang libncurses5-dev libncursesw5-dev zlib1g-dev pkg-config libssl-dev llvm
```

Mac 사용자:

```sh
xcode-select --install
brew install --with-toolchain llvm
brew install pkg-config
brew install openssl
```

## 빌드 순서

```sh
git clone https://github.com/mimblewimble/grin.git
cd grin
cargo build --release
```

Grin은 Debug 모드로 Build 할 수 있습니다. (`--release` 플래그가 사용하지 않고, `--debug` 또는 `--verbose` 플래그를 사용하세요.) 그러나 이 명령어는 암호 오퍼레이션으로 인해 큰 오버헤드를 가지므로 fast sync 가 어려울 정도로 느려집니다.

## Build 에러들

[트러블 슈팅 관련해서는 이 링크를 클릭하세요.](https://github.com/mimblewimble/docs/wiki/Troubleshooting)

## 무엇을 Build 해야 되나요?

성공적으로 빌드한다면:

* `target/release/grin` - 메인 grin 바이너리 디렉토리가 생성됩니다.

모든 데이터, 설정, 로그 파일들은 기본적으로 숨겨진 `~/.grin` 디렉토리에 생성되고 사용됩니다. (user home 디렉토리 안에 있습니다.)
`~/.grin/main/grin-server.toml` 을 수정해서 모든 설정값들을 바꿀 수 있습니다.

Grin은 현재 디렉토리 내에서도 데이터 파일들을 만들 수 있습니다. 밑에 있는 Bash 명령어를 작동하세요.

```sh
grin server config
```

이 명령어는 `grin-server.toml`를 현재 디렉토리에서 생성합니다.
이 파일은 현재 디렉토리 내의 모든 데이터에 대해서 사용하도록 미리 구성되어 있습니다.
`grin-server.toml` 파일이 있는 디렉토리에서 grin을 실행하면 기본값`~ / .grin / main / grin-server.toml` 대신 그 파일의 값을 사용하게됩니다.

Testing 중에서는 Grin 바이너리를 이렇게 path 에 삽입 할 수도 있습니다.

```sh
export PATH=`pwd`/target/release:$PATH
```

만약 Grin을 root 디렉토리에서 실행한다고 가정하면, `grin` 명령어를 바로 실행할 수 있습니다. (`grin help` 명령어를 통해서 좀 더 많은 옵션을 알아보세요.)

## 설정하기

Grin 은 기본적으로 설정되어 있는 올바른 값으로 실행하고 `grin-server.toml`를 통해 추가로 설정하는 것이 가능합니다.
Grin이 처음 실행될때 설정파일이 생성되고 각 사용가능한 옵션에 대한 매뉴얼을 포함하고 있습니다.

`grin-server.toml` 파일을 통해 모든 Grin 서버 구성을 수행하는 것이 좋지만,
커맨드 라인 명령어를 사용하면 `grin-server.toml` 파일의 모든설정을 덮어쓰는 것이 가능합니다.

Grin을 작동시키는 명령어에 대한 도움말은 다음 명령어를 실행하세요.

```sh
grin help
grin server --help
grin client --help
```

## Docker 사용하기

```sh
docker build -t grin -f etc/Dockerfile .
```

testnet을 사용하려면 `etc/Dockerfile.testnet` 을 사용하세요.
container 안에서 grin cache를 bind-mount로 사용 할 수 있습니다.

```sh
docker run -it -d -v $HOME/.grin:/root/.grin grin
```

Docker를 named volume으로 사용하는 것을 선호한다면 `-v dotgrin:/root/.grin` 명령어를 대신 사용할 수 있습니다.
named volume샤용시 volume 생성시 기본 설정을 복사합니다.

## 크로스 플랫폼 빌드

Rust(Cargo)는 여러 플랫폼에서 Grin을 빌드 할 수 있습니다. 그래서 이론적으로 낮은 성능의 디바이스 에서도 인증받은 노드로 grin을 아마도 작동 시킬 수 있을것 입니다.
예를 들자면 라즈베리 파이 같은 x96 리눅스플랫폼 위에서 `grin` 크로스 컴파일을 하고 ARM 바이너릐를 만듭니다.

## Grin 사용하기

[지갑 유저 가이드](https://github.com/mimblewimble/docs/wiki/Wallet-User-Guide) 위키페이지와 링크된 페이지들은 어떤 Feature 를 가지고 있는지 , 트러블 슈팅 등등에 대한 좀 더 많은 정보를 가지고 있습니다.

## Grin 채굴하기

Grin의 모든 마이닝 기능은 분리된 독랍형 패키지인 [grin-miner](https://github.com/mimblewimble/grin-miner)로 옮겨졌습니다.
일단 Grin 노드가 실행되면 실행중인 노드에 대해 grin-miner를 빌드하고 실행하여 마이닝을 시작할 수 있습니다.

grin-miner가 grin 노드와 통신 할 수 있게 하려면, `grin-server.toml` 설정 파일에서`enable_stratum_server = true`가 설정되어 있는지 확인하세요. 그 다음 Wallet listener인 `grin-wallet listen` 명령어를 실행하세요 .
