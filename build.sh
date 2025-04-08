#! /usr/bin/env sh

set -e
cargo build --release
zip target/i686-pc-windows-msvc/release/zerosplitter.zip target/i686-pc-windows-msvc/release/zerosplitter.exe target/i686-pc-windows-msvc/release/payload.dll