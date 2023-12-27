#![no_std]
#![no_main]

extern crate alloc;
extern crate shared_lib;

use shared_lib::BootInfo;
use shared_lib::entry_point;
use rust_os::memory::{active_level_4_table, FrameAllocator};
use rust_os::allocator::init_heap;

use core::panic::PanicInfo;
use shared_lib::logger;
use core::arch::asm;
use rust_os::task::executor::Executor;
use rust_os::task::{keyboard, Task};

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

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static shared_lib::BootInfo) -> ! {
    let fb_info = boot_info.fb_info;
    let memory_map = &boot_info.memory_map;

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(fb_info));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Debug);

    rust_os::init();

    log::info!("Hello from kernel!");
    shared_lib::serial_println!("Hello from kernel!");

    let l4_table = unsafe {
        active_level_4_table()
    };

    let mut allocator = FrameAllocator::new(memory_map);

    init_heap(l4_table, &mut allocator)
        .expect("Failed to init heap");

    let mut executor = Executor::new();
    executor.spawn(Task::new(ok_task()));
    executor.spawn(Task::new(keyboard::print_keypresses()));

    executor.run();
}

async fn async_number() -> u32 {
    42
}

async fn ok_task() {
    let number = async_number().await;
    log::info!("Everything is ok. async number: {}", number);
}
