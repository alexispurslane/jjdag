build:
    cargo build --release

check:
    cargo clippy --all-targets --all-features

fmt:
    cargo fmt --all

run *args:
    cargo run --release -- {{args}}
