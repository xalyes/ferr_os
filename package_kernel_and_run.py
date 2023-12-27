#!/usr/bin/env python3

import os
import sys
import subprocess

current_dir = os.path.dirname(os.path.realpath(__file__))

disk_image_result = subprocess.run(["cargo", "+stable", "run",
"--package", "disk_image",
"--target", "x86_64-unknown-linux-gnu",
"--", current_dir + "/target/x86_64-unknown-uefi/debug/loader.efi", sys.argv[1]])

if disk_image_result.returncode != 0:
    sys.exit(1)

kernel_result = subprocess.run(["qemu-system-x86_64", "-drive", "format=raw,file=" + current_dir + "/target/x86_64-unknown-uefi/debug/loader.gdt",
"-bios", current_dir + "/build/OVMF_CODE.fd", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04", "-serial", "stdio", "-no-reboot"] + sys.argv[2:])

if kernel_result.returncode == 33:
    sys.exit(0)
else:
    sys.exit(1)