use core::arch::asm;
use bitflags::bitflags;
use lazy_static::lazy_static;
use shared_lib::bits::{get_bits, set_bits};
use shared_lib::addr::VirtAddr;

#[derive(Debug, Clone, Copy)]
#[repr(C, packed(4))]
pub struct TaskStateSegment {
    reserved_1: u32,
    /// The full 64-bit canonical forms of the stack pointers (RSP) for privilege levels 0-2.
    pub privilege_stack_table: [VirtAddr; 3],
    reserved_2: u64,
    /// The full 64-bit canonical forms of the interrupt stack table (IST) pointers.
    pub interrupt_stack_table: [VirtAddr; 7],
    reserved_3: u64,
    reserved_4: u16,
    /// The 16-bit offset to the I/O permission bit map from the 64-bit TSS base.
    pub iomap_base: u16,
}

impl TaskStateSegment {
    /// Creates a new TSS with zeroed privilege and interrupt stack table and an
    /// empty I/O-Permission Bitmap.
    ///
    /// As we always set the TSS segment limit to
    /// `size_of::<TaskStateSegment>() - 1`, this means that `iomap_base` is
    /// initialized to `size_of::<TaskStateSegment>()`.
    #[inline]
    pub const fn new() -> TaskStateSegment {
        TaskStateSegment {
            privilege_stack_table: [VirtAddr::zero(); 3],
            interrupt_stack_table: [VirtAddr::zero(); 7],
            iomap_base: core::mem::size_of::<TaskStateSegment>() as u16,
            reserved_1: 0,
            reserved_2: 0,
            reserved_3: 0,
            reserved_4: 0,
        }
    }
}

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

lazy_static! {
    static ref TSS: TaskStateSegment = {
        let mut tss = TaskStateSegment::new();
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

            let stack_start = VirtAddr::from_ptr(unsafe { &STACK });
            let stack_end = VirtAddr::new(stack_start.0 + STACK_SIZE as u64);
            stack_end
        };
        tss
    };
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PrivilegeLevel {
    /// Privilege-level 0 (most privilege): This level is used by critical system-software
    /// components that require direct access to, and control over, all processor and system
    /// resources. This can include BIOS, memory-management functions, and interrupt handlers.
    Ring0 = 0,

    /// Privilege-level 1 (moderate privilege): This level is used by less-critical system-
    /// software services that can access and control a limited scope of processor and system
    /// resources. Software running at these privilege levels might include some device drivers
    /// and library routines. The actual privileges of this level are defined by the
    /// operating system.
    Ring1 = 1,

    /// Privilege-level 2 (moderate privilege): Like level 1, this level is used by
    /// less-critical system-software services that can access and control a limited scope of
    /// processor and system resources. The actual privileges of this level are defined by the
    /// operating system.
    Ring2 = 2,

    /// Privilege-level 3 (least privilege): This level is used by application software.
    /// Software running at privilege-level 3 is normally prevented from directly accessing
    /// most processor and system resources. Instead, applications request access to the
    /// protected processor and system resources by calling more-privileged service routines
    /// to perform the accesses.
    Ring3 = 3,
}

impl PrivilegeLevel {
    /// Creates a `PrivilegeLevel` from a numeric value. The value must be in the range 0..4.
    ///
    /// This function panics if the passed value is >3.
    #[inline]
    pub const fn from_u16(value: u16) -> PrivilegeLevel {
        match value {
            0 => PrivilegeLevel::Ring0,
            1 => PrivilegeLevel::Ring1,
            2 => PrivilegeLevel::Ring2,
            3 => PrivilegeLevel::Ring3,
            _ => panic!("invalid privilege level"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct SegmentSelector(pub u16);

impl SegmentSelector {
    /// Creates a new SegmentSelector
    ///
    /// # Arguments
    ///  * `index`: index in GDT or LDT array (not the offset)
    ///  * `rpl`: the requested privilege level
    #[inline]
    pub const fn new(index: u16, rpl: PrivilegeLevel) -> SegmentSelector {
        SegmentSelector(index << 3 | (rpl as u16))
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed(2))]
pub struct DescriptorTablePointer {
    /// Size of the DT.
    pub limit: u16,
    /// Pointer to the memory region containing the DT.
    pub base: VirtAddr,
}

#[derive(Debug, Clone)]
pub struct GlobalDescriptorTable {
    table: [u64; 8],
    len: usize,
}

impl GlobalDescriptorTable {
    /// Creates an empty GDT.
    #[inline]
    pub const fn new() -> GlobalDescriptorTable {
        GlobalDescriptorTable {
            table: [0; 8],
            len: 1,
        }
    }

    #[inline]
    fn push(&mut self, value: u64) -> usize {
        let index = self.len;
        self.table[index] = value;
        self.len += 1;
        index
    }

    #[inline]
    pub fn add_entry(&mut self, entry: Descriptor) -> SegmentSelector {
        let index = match entry {
            Descriptor::UserSegment(value) => {
                if self.len > self.table.len().saturating_sub(1) {
                    panic!("GDT full")
                }
                self.push(value)
            }
            Descriptor::SystemSegment(value_low, value_high) => {
                if self.len > self.table.len().saturating_sub(2) {
                    panic!("GDT requires two free spaces to hold a SystemSegment")
                }
                let index = self.push(value_low);
                self.push(value_high);
                index
            }
        };
        SegmentSelector::new(index as u16, entry.dpl())
    }

    #[inline]
    pub fn load(&'static self) {
        // SAFETY: static lifetime ensures no modification after loading.
        unsafe { self.load_unsafe() };
    }

    #[inline]
    pub unsafe fn load_unsafe(&self) {
        unsafe {
            asm!("lgdt [{}]", in(reg) &self.pointer(), options(readonly, nostack, preserves_flags));
        }
    }

    fn pointer(&self) -> DescriptorTablePointer {
        use core::mem::size_of;
        DescriptorTablePointer {
            base: VirtAddr::new(self.table.as_ptr() as u64),
            limit: (self.len * size_of::<u64>() - 1) as u16,
        }
    }
}

bitflags! {
    /// Flags for a GDT descriptor. Not all flags are valid for all descriptor types.
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Copy)]
    pub struct DescriptorFlags: u64 {
        /// Set by the processor if this segment has been accessed. Only cleared by software.
        /// _Setting_ this bit in software prevents GDT writes on first use.
        const ACCESSED          = 1 << 40;
        /// For 32-bit data segments, sets the segment as writable. For 32-bit code segments,
        /// sets the segment as _readable_. In 64-bit mode, ignored for all segments.
        const WRITABLE          = 1 << 41;
        /// For code segments, sets the segment as “conforming”, influencing the
        /// privilege checks that occur on control transfers. For 32-bit data segments,
        /// sets the segment as "expand down". In 64-bit mode, ignored for data segments.
        const CONFORMING        = 1 << 42;
        /// This flag must be set for code segments and unset for data segments.
        const EXECUTABLE        = 1 << 43;
        /// This flag must be set for user segments (in contrast to system segments).
        const USER_SEGMENT      = 1 << 44;
        /// These two bits encode the Descriptor Privilege Level (DPL) for this descriptor.
        /// If both bits are set, the DPL is Ring 3, if both are unset, the DPL is Ring 0.
        const DPL_RING_3        = 3 << 45;
        /// Must be set for any segment, causes a segment not present exception if not set.
        const PRESENT           = 1 << 47;
        /// Available for use by the Operating System
        const AVAILABLE         = 1 << 52;
        /// Must be set for 64-bit code segments, unset otherwise.
        const LONG_MODE         = 1 << 53;
        /// Use 32-bit (as opposed to 16-bit) operands. If [`LONG_MODE`][Self::LONG_MODE] is set,
        /// this must be unset. In 64-bit mode, ignored for data segments.
        const DEFAULT_SIZE      = 1 << 54;
        /// Limit field is scaled by 4096 bytes. In 64-bit mode, ignored for all segments.
        const GRANULARITY       = 1 << 55;

        /// Bits `0..=15` of the limit field (ignored in 64-bit mode)
        const LIMIT_0_15        = 0xFFFF;
        /// Bits `16..=19` of the limit field (ignored in 64-bit mode)
        const LIMIT_16_19       = 0xF << 48;
        /// Bits `0..=23` of the base field (ignored in 64-bit mode, except for fs and gs)
        const BASE_0_23         = 0xFF_FFFF << 16;
        /// Bits `24..=31` of the base field (ignored in 64-bit mode, except for fs and gs)
        const BASE_24_31        = 0xFF << 56;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Descriptor {
    /// Descriptor for a code or data segment.
    ///
    /// Since segmentation is no longer supported in 64-bit mode, almost all of
    /// code and data descriptors is ignored. Only some flags are still used.
    UserSegment(u64),
    /// A system segment descriptor such as a LDT or TSS descriptor.
    SystemSegment(u64, u64),
}

impl Descriptor {
    #[inline]
    pub const fn kernel_code_segment() -> Descriptor {
        Descriptor::UserSegment
            ( DescriptorFlags::USER_SEGMENT.bits()
            | DescriptorFlags::PRESENT.bits()
            | DescriptorFlags::WRITABLE.bits()
            | DescriptorFlags::ACCESSED.bits()
            | DescriptorFlags::LIMIT_0_15.bits()
            | DescriptorFlags::LIMIT_16_19.bits()
            | DescriptorFlags::GRANULARITY.bits()
            | DescriptorFlags::EXECUTABLE.bits()
            | DescriptorFlags::LONG_MODE.bits()
            )
    }

    #[inline]
    pub const fn kernel_data_segment() -> Descriptor {
        Descriptor::UserSegment
            ( DescriptorFlags::USER_SEGMENT.bits()
                | DescriptorFlags::PRESENT.bits()
                | DescriptorFlags::WRITABLE.bits()
                | DescriptorFlags::ACCESSED.bits()
                | DescriptorFlags::LIMIT_0_15.bits()
                | DescriptorFlags::LIMIT_16_19.bits()
                | DescriptorFlags::GRANULARITY.bits()
                | DescriptorFlags::DEFAULT_SIZE.bits()
            )
    }

    #[inline]
    pub fn tss_segment(tss: &'static TaskStateSegment) -> Descriptor {
        // SAFETY: The pointer is derived from a &'static reference, which ensures its validity.
        unsafe { Self::tss_segment_unchecked(tss) }
    }

    #[inline]
    pub const fn dpl(self) -> PrivilegeLevel {
        let value_low = match self {
            Descriptor::UserSegment(v) => v,
            Descriptor::SystemSegment(v, _) => v,
        };
        let dpl = (value_low & DescriptorFlags::DPL_RING_3.bits()) >> 45;
        PrivilegeLevel::from_u16(dpl as u16)
    }

    #[inline]
    pub unsafe fn tss_segment_unchecked(tss: *const TaskStateSegment) -> Descriptor {
        use self::DescriptorFlags as Flags;
        use core::mem::size_of;

        let ptr = tss as u64;

        let mut low = Flags::PRESENT.bits();

        // base
        set_bits(&mut low, get_bits(ptr, 0..24), 16);
        set_bits(&mut low, get_bits(ptr, 24..32), 56);

        // limit (the `-1` in needed since the bound is inclusive)
        set_bits(&mut low, (size_of::<TaskStateSegment>() - 1) as u64, 0);

        // type (0b1001 = available 64-bit tss)
        set_bits(&mut low, 0b1001, 40);

        let mut high = 0;
        set_bits(&mut high, get_bits(ptr, 32..64), 0);

        Descriptor::SystemSegment(low, high)
    }
}

struct GdtAndSelectors {
    pub gdt: GlobalDescriptorTable,
    pub code_selector: SegmentSelector,
    pub tss_selector: SegmentSelector,
    pub data_selector: SegmentSelector
}

lazy_static! {
    static ref GDT: GdtAndSelectors = {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
        let tss_selector = gdt.add_entry(Descriptor::tss_segment(&TSS));
        let data_selector = gdt.add_entry(Descriptor::kernel_data_segment());
        GdtAndSelectors { gdt: gdt, code_selector: code_selector, tss_selector: tss_selector, data_selector: data_selector }
    };
}

pub fn init() {
    GDT.gdt.load();

    unsafe {
        asm!(
            "push {sel}",
            "lea {tmp}, [1f + rip]",
            "push {tmp}",
            "retfq",
            "1:",
            sel = in(reg) GDT.code_selector.0 as u64,
            tmp = lateout(reg) _,
            options(preserves_flags),
        );

        asm!("mov ds, {0:x}", in(reg) GDT.data_selector.0, options(nostack, preserves_flags));
        asm!("mov es, {0:x}", in(reg) GDT.data_selector.0, options(nostack, preserves_flags));
        asm!("mov ss, {0:x}", in(reg) GDT.data_selector.0, options(nostack, preserves_flags));

        asm!("ltr {0:x}", in(reg) GDT.tss_selector.0, options(nostack, preserves_flags));
    }

}