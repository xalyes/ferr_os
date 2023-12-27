use core::ops::{Deref, DerefMut};
use crate::addr::VirtAddr;
use crate::page_table::{PageTable, PageTablesAllocator};

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum MemoryType {
    Free,
    Reserved,
    InUse,
    Acpi1_3,
    AcpiReclaim,
    Acpi1_4,
}

#[derive(Copy, Clone)]
pub struct MemoryRegion {
    pub ty: MemoryType,
    pub addr: u64,
    pub page_count: usize
}

pub const MAX_MEMORY_MAP_SIZE: usize = 256;
pub const MEMORY_MAP_PAGES: usize = 1 + (core::mem::size_of::<MemoryRegion>() * MAX_MEMORY_MAP_SIZE) / 4096;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryMap {
    pub entries: [MemoryRegion; MAX_MEMORY_MAP_SIZE],
    pub next_free_entry_idx: u64
}

impl MemoryMap {
    fn next_free_entry_index(&self) -> usize {
        self.next_free_entry_idx as usize
    }
}

impl Deref for MemoryMap {
    type Target = [MemoryRegion];

    fn deref(&self) -> &Self::Target {
        &self.entries[0..self.next_free_entry_index()]
    }
}

impl DerefMut for MemoryMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let next_index = self.next_free_entry_index();
        &mut self.entries[0..next_index]
    }
}

#[repr(align(4096))]
pub struct FrameAllocator {
    memory_map: &'static MemoryMap,
    pub next: usize,
    mapping_offset: u64
}

impl FrameAllocator {
    pub fn new(memory_map: &'static MemoryMap, mapping_offset: u64, next_free_frame: usize) -> Self {
        FrameAllocator {
            memory_map,
            next: next_free_frame,
            mapping_offset
        }
    }

    fn usable_frames(&self) -> impl Iterator<Item = u64> + '_ {
        // get usable regions from memory map
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.ty == MemoryType::Free);

        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.addr..(r.addr + 4096 * r.page_count as u64));

        // transform to an iterator of frame start addresses
        addr_ranges.flat_map(|r| r.step_by(4096))
    }

    pub fn allocate_frame(&mut self) -> Option<u64> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}

impl PageTablesAllocator for FrameAllocator {
    fn allocate_page_table(&mut self) -> Result::<&mut PageTable, &'static str> {
        let frame = self.allocate_frame().expect("Out of memory - failed to allocate frame");

        log::info!("Allocated page table. Addr: {:#x}", frame);
        let page = VirtAddr::new_checked(frame + self.mapping_offset)
            .expect("Failed to create virt address");

        let page_table = unsafe { core::slice::from_raw_parts_mut(page.0 as *mut PageTable, 4096) };
        page_table[0].clear();
        Ok(&mut page_table[0])
    }
}