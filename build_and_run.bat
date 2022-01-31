cargo +nightly build -Zbuild-std --release --target x86_64-unknown-uefi^
 && cargo +nightly build -Zbuild-std=core --target kernel/x86_64-default_settings.json --release --manifest-path kernel/Cargo.toml^
 && xcopy /y target\x86_64-unknown-uefi\release\uefi_bootloader.efi build\EFI\BOOT\BOOTX64.EFI*^
 && xcopy /y target\x86_64-default_settings\release\kernel build\kernel*^
 && qemu-system-x86_64 -nodefaults -vga std -machine q35,accel=kvm:tcg -m 128M -drive "if=pflash,format=raw,readonly,file=build/OVMF_CODE.fd" -drive "if=pflash,format=raw,file=build/OVMF_VARS.fd" -drive "format=raw,file=fat:rw:build" -serial stdio -monitor vc:2560x1440 -d mmu -no-reboot -no-shutdown
