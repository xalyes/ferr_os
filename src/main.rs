#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(shared_lib::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate shared_lib;

use rust_os::entry_point;
use rust_os::memory::{active_level_4_table, Allocator, translate_addr};

use core::panic::PanicInfo;
use shared_lib::{logger, VIRT_MAPPING_OFFSET};

#[cfg(not(test))]
use core::arch::asm;
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

    for i in 0..512 {
        if l4_table[i].is_present() {
            log::info!("L4 Entry {}: {:#x}", i, l4_table[i].addr());
        }
    }

    let addresses = [
        // framebuffer page
        0x180_8000_0000,
        // some code page
        0x201008,
        // some stack page
        0x5_0000,
        // virtual address mapped to physical address 0
        VIRT_MAPPING_OFFSET,
    ];

    for &addr in &addresses {
        let virt = VirtAddr::new_checked(addr).unwrap();
        let phys = unsafe { translate_addr(virt) };

        log::info!("{} -> {:#x}", virt, phys.unwrap());
    }

    let mut allocator = Allocator::new(memory_map);
    let page = VirtAddr::new(0);
    let frame = 0x_8000_0000;

    unsafe {
        map_address_with_offset(&mut l4_table, page, frame, &mut allocator, VIRT_MAPPING_OFFSET)
            .expect("Failed to map");
    }

    log::info!("Mapped!");

    unsafe {
        log::info!("Translated {} -> {:#x}", VirtAddr::new(0xc80), translate_addr(VirtAddr::new(0xc80)).unwrap());
    }

    let page_ptr = page.0 as *mut u64;
    unsafe {
        page_ptr.offset(400).write_volatile(0x_f021_f077_f065_f04e);
    }

    log::info!("Everything is ok");

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
