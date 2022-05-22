# Build Stage
FROM ubuntu:20.04 as builder

## Install build dependencies.
RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y cmake clang curl git-all build-essential binutils-dev libunwind-dev libblocksruntime-dev liblzma-dev
RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
RUN ${HOME}/.cargo/bin/rustup default nightly
RUN ${HOME}/.cargo/bin/cargo install honggfuzz
RUN git clone https://github.com/rust-blockchain/evm.git
WORKDIR /evm/fuzzer
RUN ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /
COPY Mayhemfile Mayhemfile
FROM ubuntu:20.04

COPY --from=builder /evm/fuzzer/hfuzz_target/x86_64-unknown-linux-gnu/release/* /
COPY --from=builder /Mayhemfile /