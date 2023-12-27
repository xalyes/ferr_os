cargo build --package loader --target x86_64-unknown-uefi -Z build-std=core -Z build-std-features=compiler-builtins-mem &&
cargo test --release --workspace --exclude disk_image --exclude loader -- -display none
