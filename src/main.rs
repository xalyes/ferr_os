#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(rust_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate shared_lib;

use rust_os::entry_point;

mod serial;

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}

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
        unsafe { asm!("cli; hlt") };
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
    log::set_max_level(log::LevelFilter::Info);

    serial_println!("Hello from kernel!");
    log::info!("Hello from kernel!");

    rust_os::init();

    unsafe {
        asm!("int3", options(nomem, nostack));
    }

    log::info!("Everything is ok");

    loop {}
}
