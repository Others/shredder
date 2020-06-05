#!/usr/bin/env fish
cat (status -f)
cargo fmt;and cargo clippy;and cargo audit;and cargo outdated;and cargo test --release
