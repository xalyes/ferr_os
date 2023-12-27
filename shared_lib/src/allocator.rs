use core::ops::{Deref, DerefMut};
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
pub struct MemoryMap {
    pub entries: &'static mut [MemoryRegion; MAX_MEMORY_MAP_SIZE],
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
pub struct Allocator {
    pub memory_map: MemoryMap
}

impl Allocator {
    pub fn new(memory_map: MemoryMap) -> Self {
        Allocator {
            memory_map
        }
    }

    pub fn allocate(&mut self, count: usize) -> Result<u64, &'static str> {
        let memory_map = &mut self.memory_map;

        for region in memory_map.iter_mut() {
            match &region.ty {
                MemoryType::Free => {
                    if region.page_count < count {
                        continue;
                    }

                    let addr = if region.page_count == count {
                        region.ty = MemoryType::InUse;
                        region.addr
                    } else {
                        region.page_count -= count;
                        let new_region_addr = region.addr + (region.page_count * 4096) as u64;
                        memory_map.entries[memory_map.next_free_entry_idx as usize] = MemoryRegion {
                            ty: MemoryType::InUse,
                            addr: new_region_addr,
                            page_count: count
                        };
                        memory_map.next_free_entry_idx += 1;
                        new_region_addr
                    };
                    log::info!("Allocated region for {} pages. Addr: {:#x}", count, addr);
                    return Ok(addr);
                }
                _other => {}
            }
        }
        Err("Out of memory!")
    }
}

impl PageTablesAllocator for Allocator {
    fn allocate_page_table(&mut self) -> Result::<&mut PageTable, &'static str> {
        let memory_map = &mut self.memory_map;

        for region in memory_map.iter_mut() {
            match &region.ty {
                MemoryType::Free => {
                    let addr = if region.page_count == 1 {
                        region.ty = MemoryType::InUse;
                        region.addr
                    } else {
                        region.page_count -= 1;
                        let new_region_addr = region.addr + (region.page_count * 4096) as u64;
                        memory_map.entries[memory_map.next_free_entry_idx as usize] = MemoryRegion {
                            ty: MemoryType::InUse,
                            addr: new_region_addr,
                            page_count: 1
                        };
                        memory_map.next_free_entry_idx += 1;
                        new_region_addr
                    };
                    log::trace!("Allocated page table. Addr: {:#x}", addr);
                    let page_table = unsafe { core::slice::from_raw_parts_mut(addr as *mut PageTable, 4096) };
                    page_table[0].clear();
                    return Ok(&mut page_table[0]);
                }
                _other => {}
            }
        }
        Err("Out of memory!")
    }
}