#![no_std]
#![feature(abi_x86_interrupt)]
#![feature(const_mut_refs)]

extern crate alloc;
use core::arch::asm;
use core::panic::PanicInfo;
use shared_lib::frame_allocator::FrameAllocator;
use shared_lib::serial_println;
use crate::apic::{disable_pic, initialize_apic};
use crate::xsdt::read_xsdt;

pub mod idt;
mod interrupts;
pub mod gdt;
pub mod port;
pub mod memory;
pub mod task;
pub mod allocator;
pub mod shell;
mod apic;
mod xsdt;
mod pci;
mod ide;
pub mod chrono;

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

pub fn preinit(allocator: &mut FrameAllocator, rsdp_addr: u64) {
    gdt::init();
    interrupts::init_idt();
    let apic_addrs= read_xsdt(allocator, rsdp_addr);
    disable_pic();
    initialize_apic(apic_addrs);
}

pub async fn init() {
    pci::init_pci().await;
}