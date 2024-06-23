cargo build --target x86_64-default_settings.json --release^
 && xcopy /y target\x86_64-default_settings\release\ferr_os build\kernel^
 && cargo build --package loader --target x86_64-unknown-uefi -Z build-std-features=compiler-builtins-mem --release^
 && python ./package_kernel_and_run.py build/kernel -d int -D log.txt -monitor vc:1024x768 -m 128M 
 
