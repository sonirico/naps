check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

wasm-fixture:
    rustup target add wasm32-unknown-unknown
    cargo build --release --target wasm32-unknown-unknown --manifest-path tests/fixtures/wasm-recv/Cargo.toml
    cp tests/fixtures/wasm-recv/target/wasm32-unknown-unknown/release/wasm_recv.wasm tests/fixtures/scripts/recv.wasm
