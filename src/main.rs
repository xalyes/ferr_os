#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(shared_lib::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
extern crate shared_lib;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use rust_os::{allocator, entry_point};
use rust_os::memory::{active_level_4_table, FrameAllocator, translate_addr};
use rust_os::allocator::{HEAP_SIZE, init_heap};

use core::panic::PanicInfo;
use shared_lib::{logger, VIRT_MAPPING_OFFSET};

#[cfg(not(test))]
use core::arch::asm;
use core::ops::Not;
use shared_lib::addr::VirtAddr;
use shared_lib::page_table::{map_address, map_address_with_offset};

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
    let memory_map = &mut boot_info.memory_map;

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(fb_info));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

    shared_lib::serial_println!("Hello from kernel!");
    log::info!("Hello from kernel!");

    rust_os::init();

    let mut l4_table = unsafe {
        active_level_4_table()
    };

    let mut allocator = FrameAllocator::new(memory_map);

    init_heap(l4_table, &mut allocator)
        .expect("Failed to init heap");

    let heap_value_1 = Box::new(41);
    let heap_value_2 = Box::new(13);
    assert_eq!(*heap_value_1, 41);
    assert_eq!(*heap_value_2, 13);

    let n = 1000;
    let mut vec = Vec::new();
    for i in 0..n {
        vec.push(i);
    }
    assert_eq!(vec.iter().sum::<u64>(), (n - 1) * n / 2);

    for i in 0..HEAP_SIZE {
        let x = Box::new(i);
        assert_eq!(*x, i);
    }

    let long_lived = Box::new(1); // new
    for i in 0..HEAP_SIZE {
        let x = Box::new(i);
        assert_eq!(*x, i);
    }
    assert_eq!(*long_lived, 1); // new

    log::info!("Everything is ok");

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
