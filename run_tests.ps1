cargo build --package loader --target x86_64-unknown-uefi -Z build-std-features=compiler-builtins-mem &&
cargo --config "target.'cfg(all())'.runner = 'python $PWD\package_kernel_and_run.py'" test --release -p shared_lib -p rust_os -- -display none
