use core::arch::asm;
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