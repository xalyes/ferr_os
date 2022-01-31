#![no_std] // don't link the Rust standard library
#![no_main] // disable all Rust-level entry points

use uefi_bootloader::BootInfo;
use core::{arch::asm, panic::PanicInfo};
use core::borrow::{Borrow, BorrowMut};
use uefi_bootloader::boot_info::FrameBuffer;
use uefi_bootloader::logger::{self, LOGGER};


/// Defines the entry point function.
///
/// The function must have the signature `fn(&'static mut BootInfo) -> !`.
///
/// This macro just creates a function named `_start`, which the linker will use as the entry
/// point. The advantage of using this macro instead of providing an own `_start` function is
/// that the macro ensures that the function and argument types are correct.
#[macro_export]
macro_rules! entry_point {
    ($path:path) => {
        #[export_name = "_start"]
        pub extern "C" fn __impl_start(boot_info: &'static mut $crate::BootInfo) -> ! {
            // validate the signature of the program entry point
            let f: fn(&'static mut $crate::BootInfo) -> ! = $path;

            f(boot_info)
        }
    };
}

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let fb = boot_info.framebuffer.as_mut().unwrap();

    //let mut fb : &'static mut FrameBuffer = &mut boot_info.framebuffer.into_option().expect("No framebuffer :(");
    let fb_info = fb.info.clone();

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(fb.buffer_mut(), fb_info));
    log::set_logger(logger).expect("logger already set");
    log::set_max_level(log::LevelFilter::Trace);
    log::info!("Hello from kernel");

    loop {
        unsafe { asm!("hlt") };
    }
}

/// This function is called on panic.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    loop {}
}
