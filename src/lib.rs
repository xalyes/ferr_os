#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![test_runner(shared_lib::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::asm;
use core::panic::PanicInfo;
use shared_lib::serial_println;

pub mod idt;
mod interrupts;
pub mod gdt;
mod addr;
mod pic;

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

// our panic handler in test mode
#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

#[macro_export]
macro_rules! entry_point {
    ($path:path) => {
        #[export_name = "_start"]
        pub extern "C" fn __impl_start(fb_info: &'static mut shared_lib::logger::FrameBufferInfo) -> ! {
            // validate the signature of the program entry point
            let f: fn(&'static mut shared_lib::logger::FrameBufferInfo) -> ! = $path;

            f(fb_info)
        }
    };
}

#[cfg(test)]
entry_point!(test_kernel_main);

#[cfg(test)]
fn test_kernel_main(_fb_info: &'static mut shared_lib::logger::FrameBufferInfo) -> ! {
    init();
    test_main();
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