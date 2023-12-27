#!/usr/bin/env sh

cargo build  --target x86_64-default_settings.json --release &&
cp -f target/x86_64-default_settings/release/rust_os build/kernel &&
cargo build --package loader --target x86_64-unknown-uefi -Z build-std=core -Z build-std-features=compiler-builtins-mem &&
./package_kernel_and_run.py build/kernel -d int -D log.txt -monitor vc:2560x1440 -m 128M
