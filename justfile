export CARGO_HOME := env('CARGO_HOME', '')

CARGO := 'cargo'

build:
    {{CARGO}} build --workspace

test:
    {{CARGO}} test --workspace

fmt:
    {{CARGO}} fmt --all

clippy:
    {{CARGO}} clippy --workspace -- -D warnings

lint: fmt clippy

pre-commit: fmt clippy test

check:
    {{CARGO}} check --workspace
