#!/usr/bin/env fish
cat (status -f)
cargo fmt;and cargo clippy --all --all-targets -- -Dwarnings -Drust-2018-idioms;and cargo audit;and cargo update;and cargo outdated;and cargo test --release
