#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(const_mut_refs)]

extern crate alloc;
use core::arch::asm;
use core::panic::PanicInfo;
use shared_lib::frame_allocator::FrameAllocator;
use shared_lib::serial_println;
use crate::apic::{disable_pic, initialize_apic_timer};
use crate::xsdt::read_xsdt;

pub mod idt;
mod interrupts;
pub mod gdt;
mod pic;
pub mod memory;
pub mod task;
pub mod allocator;
mod apic;
mod xsdt;

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

pub fn init(allocator: &mut FrameAllocator, rsdp_addr: u64) {
    gdt::init();
    interrupts::init_idt();
    let local_apic = read_xsdt(allocator, rsdp_addr);
    disable_pic();
    initialize_apic_timer(local_apic);
}