use core::arch::asm;
use shared_lib::addr::VirtAddr;
use shared_lib::page_table::{PageTable, PageTablesAllocator};
use shared_lib::VIRT_MAPPING_OFFSET;

pub unsafe fn active_level_4_table() -> &'static mut shared_lib::page_table::PageTable
{
    let value: u64;

    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }

    let level_4_table_frame = value & 0x_000f_ffff_ffff_f000;

    let virt = VIRT_MAPPING_OFFSET + level_4_table_frame;
    let page_table_ptr = virt as *mut shared_lib::page_table::PageTable;

    &mut *page_table_ptr // unsafe
}

pub unsafe fn translate_addr(addr: VirtAddr) -> Option<u64> {
    translate_addr_inner(addr)
}

fn translate_addr_inner(addr: VirtAddr) -> Option<u64> {
    let table_indexes = [
        addr.p4_index(), addr.p3_index(), addr.p2_index(), addr.p1_index()
    ];

    let mut value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    let mut frame = value & 0x_000f_ffff_ffff_f000;

    for &index in &table_indexes {
        let virt = frame + VIRT_MAPPING_OFFSET;
        let table_ptr= virt as *const PageTable;
        let table = unsafe { &*table_ptr };

        let entry = table[index];
        if entry.is_present() {
            frame = entry.addr();
        } else {
            return None;
        }
    }

    Some(frame + u64::from(addr.get_page_offset()))
}

#[repr(align(4096))]
pub struct FrameAllocator {
    pub memory_map: &'static mut shared_lib::allocator::MemoryMap,
    next_free_frame: usize
}

impl FrameAllocator {
    pub fn new(memory_map: &'static mut shared_lib::allocator::MemoryMap) -> Self {
        FrameAllocator {
            memory_map,
            next_free_frame: 0,
        }
    }

    fn usable_frames(&self) -> impl Iterator<Item = u64> + '_ {
        // get usable regions from memory map
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.ty == shared_lib::allocator::MemoryType::Free);

        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.addr..(r.addr + 4096 * r.page_count as u64));

        // transform to an iterator of frame start addresses
        addr_ranges.flat_map(|r| r.step_by(4096))
    }

    pub fn allocate_frame(&mut self) -> Option<u64> {
        let frame = self.usable_frames().nth(self.next_free_frame);
        self.next_free_frame += 1;
        frame
    }
}

impl PageTablesAllocator for FrameAllocator {
    fn allocate_page_table(&mut self) -> Result::<&mut PageTable, &'static str> {
        let frame = self.allocate_frame().expect("Out of memory - failed to allocate frame");

        log::info!("Allocated page table. Addr: {:#x}", frame);
        let page = VirtAddr::new_checked(frame + VIRT_MAPPING_OFFSET)
            .expect("Failed to create virt address");

        let page_table = unsafe { core::slice::from_raw_parts_mut(page.0 as *mut PageTable, 4096) };
        page_table[0].clear();
        Ok(&mut page_table[0])
    }
}