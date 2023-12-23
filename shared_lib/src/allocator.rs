use crate::page_table::{PageTable, PageTablesAllocator};

#[derive(Debug)]
pub enum MemoryType {
    Free,
    Reserved,
    InUse,
    Acpi1_3,
    AcpiReclaim,
    Acpi1_4,
}

pub struct MemoryRegion {
    pub ty: MemoryType,
    pub addr: u64,
    pub page_count: usize
}

#[repr(align(4096))]
pub struct Allocator<'a> {
    pub memory_map: &'a mut [MemoryRegion],
    size: usize
}

impl<'a> Allocator<'a> {
    pub fn new(memory_map: &'a mut [MemoryRegion], size: usize) -> Self {
        Allocator {
            memory_map,
            size
        }
    }
}

impl PageTablesAllocator for Allocator<'_> {
    fn allocate_page_table(&mut self) -> Result::<&mut PageTable, &'static str> {
        for region in self.memory_map.iter_mut() {
            match &region.ty {
                MemoryType::Free => {
                    let addr = if region.page_count == 1 {
                        region.ty = MemoryType::InUse;
                        region.addr
                    } else {
                        region.page_count -= 1;
                        let new_region_addr = region.addr + (region.page_count * 4096) as u64;
                        self.memory_map[self.size] = MemoryRegion {
                            ty: MemoryType::InUse,
                            addr: new_region_addr,
                            page_count: 1
                        };
                        self.size += 1;
                        new_region_addr
                    };
                    log::info!("Allocated page table. Addr: {:#x}", addr);
                    let page_table = unsafe { core::slice::from_raw_parts_mut(addr as *mut PageTable, 4096) };
                    page_table[0].clear();
                    return Ok(&mut page_table[0]);
                }
                _other => {}
            }
        }
        Err("Out of memory!")
    }

    fn allocate(&mut self, count: usize) -> Result<u64, &'static str> {
        for region in self.memory_map.iter_mut() {
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
                        self.memory_map[self.size] = MemoryRegion {
                            ty: MemoryType::InUse,
                            addr: new_region_addr,
                            page_count: count
                        };
                        self.size += 1;
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