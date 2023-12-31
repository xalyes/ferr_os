#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(const_mut_refs)]

extern crate alloc;
use core::arch::asm;
use core::panic::PanicInfo;
use shared_lib::serial_println;

pub mod idt;
mod interrupts;
pub mod gdt;
mod pic;
pub mod memory;
pub mod task;
pub mod allocator;

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    shared_lib::exit_qemu(shared_lib::QemuExitCode::Failed);
    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

pub fn init() {
    gdt::init();
    interrupts::init_idt();

    unsafe { interrupts::PICS.lock().initialize(); };

    // Enable hardware interrupts
    unsafe { asm!("sti", options(nomem, nostack)); }
}