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
fn test_kernel_main(_boot_info: &'static mut shared_lib::BootInfo) -> ! {
    test_main();
    loop {}
}

#[cfg(not(test))]
entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(boot_info: &'static mut shared_lib::BootInfo) -> ! {
    let fb_info = boot_info.fb_info;
    let memory_map = &boot_info.memory_map;

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(fb_info));
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

    log::info!("MMap Entries addr: {:#x}", memory_map.entries as *const _ as u64);
    log::info!("Memory map size: {}", memory_map.next_free_entry_idx - 1);
    for i in 0..5 {
        log::info!("Entry: addr: {:#x} pages: {}", memory_map.entries[i].addr, memory_map.entries[i].page_count);
    }

    log::info!("Everything is ok");

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
