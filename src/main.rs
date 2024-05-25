#![no_std]
#![no_main]

#![feature(core_intrinsics)]
extern crate alloc;
extern crate shared_lib;

use shared_lib::{BootInfo, serial_logger, VIRT_MAPPING_OFFSET};
use shared_lib::entry_point;
use rust_os::memory::active_level_4_table;

use core::panic::PanicInfo;
use shared_lib::logger;
use core::arch::asm;
use rust_os::allocator::init_heap;
use rust_os::shell::Shell;
use rust_os::task::executor::Executor;
use rust_os::task::{keyboard, Task, timer};
use rust_os::port::Port;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        logger::LOGGER
            .get()
            .map(|l| l.force_unlock());

        serial_logger::SERIAL_LOGGER
            .get()
            .map(|l| l.force_unlock());
    };

    log::error!("{}", info);

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static shared_lib::BootInfo) -> ! {
    shared_lib::serial_println!("Hello from kernel!");
    let fb_info = boot_info.fb_info;
    let memory_map = &boot_info.memory_map;

    shared_lib::serial_println!("Creating allocator");
    let l4_table = unsafe {
        active_level_4_table()
    };

    let mut allocator = shared_lib::frame_allocator::FrameAllocator::new(memory_map, VIRT_MAPPING_OFFSET, boot_info.memory_map_next_free_frame);

    shared_lib::serial_println!("Creating heap");
    init_heap(l4_table, &mut allocator)
        .expect("Failed to init heap");

    shared_lib::serial_println!("Creating logger");

    let logger_is_serial = true;

    if logger_is_serial {
        let logger = serial_logger::SERIAL_LOGGER.get_or_init(move || serial_logger::LockedSerialLogger::new());
        log::set_logger(logger).unwrap();
    } else {
        let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(fb_info));
        log::set_logger(logger).unwrap();
    }

    log::set_max_level(log::LevelFilter::Debug);

    log::info!("Hello from kernel!");

    rust_os::init(&mut allocator, boot_info.rsdp_addr);

    let mut executor: Executor = Executor::new();
    executor.spawn(Task::new(ok_task()));
    executor.spawn(Task::new(timer::timer_loop()));

    let shell = Shell::new(fb_info);
    executor.spawn(Task::new(keyboard::print_keypresses(shell)));

    executor.run();

    // TODO: ACPI shutdown
    log::info!("exited");

    let mut shutdown_port = Port::new(0xB004);
    unsafe { shutdown_port.write_u16(0x2000); };

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

async fn async_number() -> u32 {
    42
}

async fn ok_task() {
    let number = async_number().await;
    log::info!("Everything is ok. async number: {}", number);
}
