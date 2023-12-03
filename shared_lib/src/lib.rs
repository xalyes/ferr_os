#![no_std]

pub mod logger;

use bitflags::bitflags;

pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry {
    entry: u64,
}

impl PageTableEntry {
    #[inline]
    pub const fn new() -> Self {
        PageTableEntry { entry: 0 }
    }

    #[inline]
    pub fn set_addr(&mut self, addr: u64, flags: PageTableFlags) {
        self.entry = addr | flags.bits();
    }

    /// Returns the flags of this entry.
    #[inline]
    pub const fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.entry)
    }

    /// Returns the physical address mapped by this entry, might be zero.
    #[inline]
    pub fn addr(&self) -> u64 {
        self.entry & 0x000f_ffff_ffff_f000
    }
}

bitflags! {
    /// Possible flags for a page table entry.
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
    pub struct PageTableFlags: u64 {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT =         1;
        /// Controls whether writes to the mapped frames are allowed.
        ///
        /// If this bit is unset in a level 1 page table entry, the mapped frame is read-only.
        /// If this bit is unset in a higher level page table entry the complete range of mapped
        /// pages is read-only.
        const WRITABLE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
        const USER_ACCESSIBLE = 1 << 2;
        /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
        /// policy is used.
        const WRITE_THROUGH =   1 << 3;
        /// Disables caching for the pointed entry is cacheable.
        const NO_CACHE =        1 << 4;
        /// Set by the CPU when the mapped frame or page table is accessed.
        const ACCESSED =        1 << 5;
        /// Set by the CPU on a write to the mapped frame.
        const DIRTY =           1 << 6;
        /// Specifies that the entry maps a huge frame instead of a page table. Only allowed in
        /// P2 or P3 tables.
        const HUGE_PAGE =       1 << 7;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_9 =           1 << 9;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_10 =          1 << 10;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_11 =          1 << 11;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_52 =          1 << 52;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_53 =          1 << 53;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_54 =          1 << 54;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_55 =          1 << 55;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_56 =          1 << 56;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_57 =          1 << 57;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_58 =          1 << 58;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_59 =          1 << 59;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_60 =          1 << 60;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_61 =          1 << 61;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_62 =          1 << 62;
        /// Forbid code execution from the mapped frames.
        ///
        /// Can be only used when the no-execute page protection feature is enabled in the EFER
        /// register.
        const NO_EXECUTE =      1 << 63;
    }
}

pub const ENTRY_COUNT: u16 = 512;

#[repr(align(4096))]
#[derive(Clone, Copy)]
pub struct PageTable {
    entries: [PageTableEntry; ENTRY_COUNT as usize],
}

impl PageTable {
    pub const fn new() -> Self {
        const EMPTY: PageTableEntry = PageTableEntry::new();
        PageTable {
            entries: [EMPTY; ENTRY_COUNT as usize],
        }
    }
}

impl core::ops::Index<u16> for PageTable {
    type Output = PageTableEntry;

    #[inline]
    fn index(&self, index: u16) -> &Self::Output {
        &self.entries[index as usize]
    }
}

impl core::ops::IndexMut<u16> for PageTable {
    #[inline]
    fn index_mut(&mut self, index: u16) -> &mut Self::Output {
        &mut self.entries[index as usize]
    }
}

unsafe fn create_next_table<'a>(page_table_entry: &'a mut PageTableEntry, page_tables_allocator: *mut dyn PageTablesAllocator)
                                -> Result::<&'a mut PageTable, &'static str> {
    if page_table_entry.flags().contains(PageTableFlags::PRESENT) {
        let next_page_table = unsafe { &mut *(page_table_entry.addr() as *mut PageTable) };
        Ok(next_page_table)
    }
    else {
        let new_table = (&mut *page_tables_allocator).allocate()?;
        page_table_entry.set_addr(new_table as *const _ as u64, PageTableFlags::PRESENT);
        Ok(new_table)
    }
}

fn get_p4_index(virt: u64) -> u16 {
    let idx = (virt >> 12 >> 9 >> 9 >> 9) as u16;
    idx % ENTRY_COUNT
}

fn get_p3_index(virt: u64) -> u16 {
    let idx = (virt >> 12 >> 9 >> 9) as u16;
    idx % ENTRY_COUNT
}

fn get_p2_index(virt: u64) -> u16 {
    let idx = (virt >> 12 >> 9) as u16;
    idx % ENTRY_COUNT
}

fn get_p1_index(virt: u64) -> u16 {
    let idx = (virt >> 12) as u16;
    idx % ENTRY_COUNT
}

trait PageTablesAllocator {
    fn allocate(&mut self) -> Result::<&mut PageTable, &'static str>;

    unsafe fn get_mut_ptr(&mut self) -> *mut dyn PageTablesAllocator;
}

#[repr(align(4096))]
pub struct StaticAllocator {
    tables: [PageTable; 100],
    next_free_table: usize
}

impl StaticAllocator {
    pub const fn new() -> Self {
        StaticAllocator {
            tables: [PageTable::new(); 100],
            next_free_table: 0
        }
    }
}

impl PageTablesAllocator for StaticAllocator {
    fn allocate(&mut self) -> Result::<&mut PageTable, &'static str> {
        if self.next_free_table == 512 {
            return Err("out of page tables");
        }

        let res = &mut self.tables[self.next_free_table];
        self.next_free_table += 1;
        Ok(res)
    }

    unsafe fn get_mut_ptr(&mut self) -> *mut dyn PageTablesAllocator {
        return self as *mut dyn PageTablesAllocator;
    }
}

pub unsafe fn map_address(l4_page_table: &mut PageTable, virt: u64, phys: u64, page_tables_allocator: &mut impl PageTablesAllocator)
                      -> core::result::Result<(), &'static str> {
    let mut allocator_ptr = page_tables_allocator.get_mut_ptr();

    let l3_table = create_next_table(&mut l4_page_table[get_p4_index(virt)],allocator_ptr)?;
    let l2_table = create_next_table(&mut l3_table[get_p3_index(virt)], allocator_ptr)?;
    let l1_table = create_next_table(&mut l2_table[get_p3_index(virt)],allocator_ptr)?;

    let mut l1_entry = &mut l1_table[get_p1_index(virt)];
    return if l1_entry.flags().contains(PageTableFlags::PRESENT) {
        core::result::Result::Err("this virtual address already mapped to frame")
    } else {
        l1_entry.set_addr(phys, PageTableFlags::PRESENT);
        Ok(())
    }
}