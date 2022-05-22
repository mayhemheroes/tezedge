# Build Stage
FROM ubuntu:20.04 as builder

## Install build dependencies.
RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y gcc cmake clang curl git-all build-essential binutils-dev libunwind-dev libblocksruntime-dev liblzma-dev
RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
RUN ${HOME}/.cargo/bin/rustup default nightly
RUN ${HOME}/.cargo/bin/cargo install honggfuzz
RUN git clone https://github.com/tezedge/tezedge.git
WORKDIR /tezedge/fuzz/ack_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/advertise_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/block_header_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/connection_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/current_branch_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/current_head_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build

WORKDIR /tezedge/fuzz/metadata_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/operation_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/operations_for_blocks_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/peer_response_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
WORKDIR /tezedge/fuzz/protocol_message/
RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build
#WORKDIR /tezedge/fuzz/encoding/
#RUN RUSTFLAGS="-Znew-llvm-pass-manager=no" HFUZZ_RUN_ARGS="--run_time $run_time --exit_upon_crash" ${HOME}/.cargo/bin/cargo hfuzz build

FROM ubuntu:20.04
COPY /Mayhem/* /Mayhem/
COPY --from=builder /tezedge/fuzz/ack_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /ack_message
COPY --from=builder /tezedge/fuzz/advertise_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /advertise_message
COPY --from=builder /tezedge/fuzz/block_header_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /block_header_message
COPY --from=builder /tezedge/fuzz/connection_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /connection_message
COPY --from=builder /tezedge/fuzz/current_branch_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /current_branch_message
COPY --from=builder /tezedge/fuzz/current_head_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /current_head_message
COPY --from=builder /tezedge/fuzz/metadata_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /metadata_message
COPY --from=builder /tezedge/fuzz/operation_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /operation_message
COPY --from=builder /tezedge/fuzz/operations_for_blocks_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /operation_for_blocks_message
COPY --from=builder /tezedge/fuzz/peer_response_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /peer_response_message
COPY --from=builder /tezedge/fuzz/protocol_message/hfuzz_target/x86_64-unknown-linux-gnu/release/ /protocol_message
