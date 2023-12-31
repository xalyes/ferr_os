pub mod fixed_size_block;

use crate::addr::VirtAddr;
use crate::page_table::{map_address_with_offset, PageTable};
use crate::VIRT_MAPPING_OFFSET;
use crate::allocator::fixed_size_block::FixedSizeBlockAllocator;
use crate::frame_allocator::FrameAllocator;

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
pub static ALLOCATOR: Locked<FixedSizeBlockAllocator> = Locked::new(FixedSizeBlockAllocator::new());

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

