#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(shared_lib::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate shared_lib;

use rust_os::entry_point;
use rust_os::memory::active_level_4_table;

use core::panic::PanicInfo;
use shared_lib::logger;

#[cfg(not(test))]
use core::arch::asm;

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        logger::LOGGER
            .get()
            .map(|l| l.force_unlock())
    };

    log::info!("{}", info);

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
    rust_os::test_panic_handler(info);
}

#[cfg(test)]
entry_point!(test_kernel_main);

#[cfg(test)]
fn test_kernel_main(_fb_info: &'static mut logger::FrameBufferInfo) -> ! {
    test_main();
    loop {}
}

#[cfg(not(test))]
entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(frame_buffer_info: &'static mut logger::FrameBufferInfo) -> ! {
    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(*frame_buffer_info));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

    shared_lib::serial_println!("Hello from kernel!");
    log::info!("Hello from kernel!");

    rust_os::init();

    let l4_table = unsafe {
        active_level_4_table()
    };

    for i in 0..512 {
        if l4_table[i].is_present() {
            log::info!("L4 Entry {}: {:#x}", i, l4_table[i].addr());
        }
    }

    log::info!("Everything is ok");

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
