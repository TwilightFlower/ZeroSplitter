#! /usr/bin/env sh

set -e
cargo build --release
cd target/i686-pc-windows-msvc/release
rm zerosplitter.zip
zip zerosplitter.zip zerosplitter.exe payload.dll