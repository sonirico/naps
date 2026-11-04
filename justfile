check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    helm lint charts/naps
    helm template charts/naps > /dev/null

wasm-fixture:
    rustup target add wasm32-unknown-unknown
    cargo build --release --target wasm32-unknown-unknown --manifest-path tests/fixtures/wasm-recv/Cargo.toml
    cp tests/fixtures/wasm-recv/target/wasm32-unknown-unknown/release/wasm_recv.wasm tests/fixtures/scripts/recv.wasm

image:
    docker build -t naps:dev .

e2e: image
    bash e2e/run.sh

readme-svg:
    python3 scripts/readme_svg.py
