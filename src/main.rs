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
use rust_os::task::executor::Executor;
use rust_os::task::{keyboard, Task};
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
    log::set_max_level(log::LevelFilter::Debug);

    rust_os::init();

    log::info!("Hello from kernel!");
    shared_lib::serial_println!("Hello from kernel!");

    let mut l4_table = unsafe {
        active_level_4_table()
    };

    let mut allocator = FrameAllocator::new(memory_map);

    init_heap(l4_table, &mut allocator)
        .expect("Failed to init heap");

    let mut executor = Executor::new();
    executor.spawn(Task::new(example_task()));
    executor.spawn(Task::new(keyboard::print_keypresses()));

    executor.run();

    log::info!("Everything is ok");

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

async fn async_number() -> u32 {
    42
}

async fn example_task() {
    let number = async_number().await;
    log::info!("async number: {}", number);
}
