#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(shared_lib::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use shared_lib::{entry_point, BootInfo, VIRT_MAPPING_OFFSET};
use core::panic::PanicInfo;
use rust_os::allocator::{HEAP_SIZE, init_heap};
use rust_os::memory::active_level_4_table;

entry_point!(main);

fn main(boot_info: &'static BootInfo) -> ! {
    use shared_lib::frame_allocator::FrameAllocator;

    let l4_table = unsafe {
        active_level_4_table()
    };

    let mut allocator = FrameAllocator::new(&boot_info.memory_map, VIRT_MAPPING_OFFSET, boot_info.memory_map_next_free_frame);

    init_heap(l4_table, &mut allocator)
        .expect("Failed to init heap");

    rust_os::init(&mut allocator, boot_info.rsdp_addr);

    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    rust_os::test_panic_handler(info)
}

#[test_case]
fn simple_allocation() {
    let heap_value_1 = Box::new(41);
    let heap_value_2 = Box::new(13);
    assert_eq!(*heap_value_1, 41);
    assert_eq!(*heap_value_2, 13);
}

#[test_case]
fn large_vec() {
    let n = 1000;
    let mut vec = Vec::new();
    for i in 0..n {
        vec.push(i);
    }
    assert_eq!(vec.iter().sum::<u64>(), (n - 1) * n / 2);
}

#[test_case]
fn many_boxes() {
    for i in 0..HEAP_SIZE {
        let x = Box::new(i);
        assert_eq!(*x, i);
    }
}

#[test_case]
fn many_boxes_long_lived() {
    let long_lived = Box::new(1);
    for i in 0..HEAP_SIZE {
        let x = Box::new(i);
        assert_eq!(*x, i);
    }
    assert_eq!(*long_lived, 1);
}