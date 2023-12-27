use core::arch::asm;
use shared_lib::addr::VirtAddr;
use shared_lib::page_table::PageTable;
use shared_lib::VIRT_MAPPING_OFFSET;

pub unsafe fn active_level_4_table() -> &'static mut PageTable
{
    let value: u64;

    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }

    let level_4_table_frame = value & 0x_000f_ffff_ffff_f000;

    let virt = VIRT_MAPPING_OFFSET + level_4_table_frame;
    let page_table_ptr = virt as *mut PageTable;

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
