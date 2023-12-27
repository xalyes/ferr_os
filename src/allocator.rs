pub mod fixed_size_block;

use shared_lib::addr::VirtAddr;
use shared_lib::page_table::{map_address_with_offset, PageTable};
use shared_lib::VIRT_MAPPING_OFFSET;
use crate::allocator::fixed_size_block::FixedSizeBlockAllocator;
use crate::memory::FrameAllocator;

pub struct Locked<A> {
    inner: spin::Mutex<A>
}

impl<A> Locked<A> {
    pub const fn new(inner: A) -> Self {
        Locked {
            inner: spin::Mutex::new(inner)
        }
    }

    pub fn lock(&self) -> spin::MutexGuard<A> {
        self.inner.lock()
    }
}

#[global_allocator]
static ALLOCATOR: Locked<FixedSizeBlockAllocator> = Locked::new(FixedSizeBlockAllocator::new());

pub const HEAP_START: usize = 0x_7777_7777_0000;
pub const HEAP_SIZE: usize = 100 * 1024; // 100 KiB

pub fn init_heap(page_table: &mut PageTable, frame_allocator: &mut FrameAllocator) -> Result<(), &'static str> {
    let mut heap = VirtAddr::new(HEAP_START as u64);
    let heap_end = heap.offset(HEAP_SIZE as u64)
        .expect("Failed to offset virtual address");

    while heap < heap_end {
        let frame = frame_allocator.allocate_frame()
            .expect("Failed to allocate frame");

        unsafe {
            map_address_with_offset(page_table, heap, frame, frame_allocator, VIRT_MAPPING_OFFSET)
                .expect("Failed to map new frame");
        }

        heap = heap.offset(4096).unwrap();
    }

    unsafe {
        ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE);
    }

    Ok(())
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

