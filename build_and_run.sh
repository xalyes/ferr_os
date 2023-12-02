#!/usr/bin/env sh

cargo build -Zbuild-std=core --target x86_64-default_settings.json --release &&
cp -f target/x86_64-default_settings/release/rust_os build/kernel &&
cargo build --package loader --target x86_64-unknown-uefi -Z build-std=core -Z build-std-features=compiler-builtins-mem &&
cargo +stable run --package disk_image --target x86_64-unknown-linux-gnu  -- target/x86_64-unknown-uefi/debug/loader.efi &&
qemu-system-x86_64 -drive format=raw,file=target/x86_64-unknown-uefi/debug/loader.gdt -bios build/OVMF_CODE.fd