#![no_main]
#![no_std]
#![feature(abi_efiapi)]
#![feature(alloc_error_handler)]
#![feature(abi_x86_interrupt)]
#![feature(maybe_uninit_slice)]

mod logger;
mod addr;
mod legacy_memory_region;
mod load_kernel;
mod level_4_entries;
mod boot_info;

extern crate alloc;

use core::{
    arch::asm,
    panic::PanicInfo,
    mem::{
        self,
        MaybeUninit
    },
    slice
};

use uefi::{
    prelude::*,
    ResultExt,
    Completion,
    proto::{
        media::{
            file::{
                FileMode,
                FileAttribute,
                RegularFile,
                File
            },
            fs::SimpleFileSystem
        },
        console::gop::{ GraphicsOutput, PixelFormat as GopPixelFormat }
    },
    table::boot::{ AllocateType, MemoryDescriptor, MemoryType }
};

use legacy_memory_region::{LegacyFrameAllocator, LegacyMemoryRegion, PAGE_SIZE};

use x86_64::{
    structures::paging::{
        FrameAllocator,
        OffsetPageTable,
        PageTable,
        PageTableFlags,
        PhysFrame,
        Size4KiB,
        Page,
        Size2MiB,
        Mapper,
        PageTableIndex
    },
    structures::gdt::{ Descriptor, GlobalDescriptorTable },
    instructions::segmentation::{ Segment, CS, DS, ES, SS },
    VirtAddr,
    PhysAddr
};

use level_4_entries::UsedLevel4Entries;

use crate::boot_info::{PixelFormat,  FrameBuffer, FrameBufferInfo, Optional, TlsTemplate, BootInfo, MemoryRegion };

/// Provides access to the page tables of the bootloader and kernel address space.
pub struct PageTables {
    /// Provides access to the page tables of the bootloader address space.
    pub bootloader: OffsetPageTable<'static>,
    /// Provides access to the page tables of the kernel address space (not active).
    pub kernel: OffsetPageTable<'static>,
    /// The physical frame where the level 4 page table of the kernel address space is stored.
    ///
    /// Must be the page table that the `kernel` field of this struct refers to.
    ///
    /// This frame is loaded into the `CR3` register on the final context switch to the kernel.
    pub kernel_level_4_frame: PhysFrame,
}

/// Required system information that should be queried from the BIOS or UEFI firmware.
#[derive(Debug, Copy, Clone)]
pub struct SystemInfo {
    /// Start address of the pixel-based framebuffer.
    pub framebuffer_addr: PhysAddr,
    /// Information about the framebuffer, including layout and pixel format.
    pub framebuffer_info: FrameBufferInfo,
    /// Address of the _Root System Description Pointer_ structure of the ACPI standard.
    pub rsdp_addr: Option<PhysAddr>,
}

#[entry]
fn efi_main(image: Handle, st: SystemTable<Boot>) -> Status {
    let (framebuffer_addr, framebuffer_info) = init_logger(&st);
    log::info!("Hello World from UEFI bootloader!");
    log::info!("Using framebuffer at {:#x}", framebuffer_addr);

    let fs = st
        .boot_services()
        .locate_protocol::<SimpleFileSystem>()
        .expect_success("failed to locate simple file system protocol");
    let fs = unsafe { &mut *fs.get() };

    let mut root = fs.open_volume().expect("Failed to open volume").unwrap();
    let handle = root.open("kernel", FileMode::Read, FileAttribute::READ_ONLY)
        .expect("Failed to open kernel file").unwrap();

    let mut file = unsafe { RegularFile::new(handle) };

    let kernel = {
        let ptr = st.boot_services().allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 10).expect("Failed to allocate page for kernel").log();
        unsafe { slice::from_raw_parts_mut(ptr as *mut u8, 40960) }
    };

    file.read(kernel).expect("Failed to read kernel file").log();

    let mmap_storage = {
        let max_mmap_size =
            st.boot_services().memory_map_size().map_size + 8 * mem::size_of::<MemoryDescriptor>();
        let ptr = st
            .boot_services()
            .allocate_pool(MemoryType::LOADER_DATA, max_mmap_size)?
            .log();
        unsafe { slice::from_raw_parts_mut(ptr, max_mmap_size) }
    };

    log::trace!("exiting boot services");
    let (system_table, memory_map) = st
        .exit_boot_services(image, mmap_storage)
        .expect_success("Failed to exit boot services");

    let mut frame_allocator = LegacyFrameAllocator::new(memory_map.copied());
    log::trace!("Frame allocator created");

    let mut page_tables = create_page_tables(&mut frame_allocator);
    log::trace!("Page tables created");

    let system_info = SystemInfo {
        framebuffer_addr,
        framebuffer_info,
        rsdp_addr: {
            use uefi::table::cfg;
            let mut config_entries = system_table.config_table().iter();
            // look for an ACPI2 RSDP first
            let acpi2_rsdp = config_entries.find(|entry| matches!(entry.guid, cfg::ACPI2_GUID));
            // if no ACPI2 RSDP is found, look for a ACPI1 RSDP
            let rsdp = acpi2_rsdp
                .or_else(|| config_entries.find(|entry| matches!(entry.guid, cfg::ACPI_GUID)));
            rsdp.map(|entry| PhysAddr::new(entry.address as u64))
        },
    };
    log::trace!("System info created");

    let mut mappings = set_up_mappings(
        &kernel,
        &mut frame_allocator,
        &mut page_tables,
        system_info.framebuffer_addr,
        system_info.framebuffer_info.byte_len,
    );
    log::trace!("Mappings setted up");

    let boot_info = create_boot_info(
        frame_allocator,
        &mut page_tables,
        &mut mappings,
        system_info,
    );
    log::trace!("Boot info created");

    switch_to_kernel(page_tables, mappings, boot_info);
}

/// Switches to the kernel address space and jumps to the kernel entry point.
pub fn switch_to_kernel(
    page_tables: PageTables,
    mappings: Mappings,
    boot_info: &'static mut crate::boot_info::BootInfo,
) -> ! {
    let PageTables {
        kernel_level_4_frame,
        ..
    } = page_tables;
    let addresses = Addresses {
        page_table: kernel_level_4_frame,
        stack_top: mappings.stack_end.start_address(),
        entry_point: mappings.entry_point,
        boot_info,
    };

    log::info!(
        "Jumping to kernel entry point at {:?}",
        addresses.entry_point
    );

    unsafe {
        context_switch(addresses);
    }
}

/*fn traverse_page_tables(level: u8, mut addr: u64)
{
    if level == 0 {
        return;
    }

    log::trace!("P{} table addr: {:#x}", level, addr);

    for n in 0..282 {
        let val: u64 = unsafe { ptr::read(addr as *const u64) };
        addr += 8;
        let present = val & 1;
        if present == 1 {
            let child_addr = val & 0xF_FFFF_FFFF_F000;

            log::trace!("Entry #{}: {:#x} -> {:#x}", n, val, child_addr);

            traverse_page_tables(level - 1, child_addr);
        }
    }
}*/

fn enable_nxe_bit() {
    let (high, low): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") 0xC000_0080 as u64,
            out("eax") low, out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    let mut msr = ((high as u64) << 32) | (low as u64);
    msr |= 1 << 11;

    let low_new = msr as u32;
    let high_new = (msr >> 32) as u32;

    unsafe {
        asm!(
            "wrmsr",
            in("ecx") 0xC000_0080 as u64,
            in("eax") low_new, in("edx") high_new,
            options(nostack, preserves_flags),
        );
    }
}

fn enable_write_protect_bit() {
    let mut value: u64;

    unsafe {
        asm!("mov {}, cr0", out(reg) value, options(nomem, nostack, preserves_flags));
    }

    value |= 1 << 16;

    unsafe {
        asm!("mov cr0, {}", in(reg) value, options(nostack, preserves_flags));
    }
}

/*fn get_cr3() -> u64
{
    let addr: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) addr, options(nomem, nostack, preserves_flags));
    }
    addr
}

fn write_cr3(addr: u64)
{
    unsafe {
        asm!("mov cr3, {}", in(reg) addr, options(nostack, preserves_flags));
    }
}*/

fn init_logger(st: &SystemTable<Boot>) -> (PhysAddr, FrameBufferInfo) {
    let gop = st
        .boot_services()
        .locate_protocol::<GraphicsOutput>()
        .expect_success("failed to locate gop");
    let gop = unsafe { &mut *gop.get() };

    let mode = {
        let modes = gop.modes().map(Completion::unwrap);
        match (
            Some(1440),
            Some(2560),
        ) {
            (Some(height), Some(width)) => modes
                .filter(|m| {
                    let res = m.info().resolution();
                    res.1 == height && res.0 == width
                })
                .last(),
            (Some(height), None) => modes.filter(|m| m.info().resolution().1 >= height).last(),
            (None, Some(width)) => modes.filter(|m| m.info().resolution().0 >= width).last(),
            _ => None,
        }
    };
    if let Some(mode) = mode {
        gop.set_mode(&mode)
            .expect_success("Failed to apply the desired display mode");
    }

    let mut resolutions_size = 0;
    let mut resolutions = [(0 as usize, 0 as usize); 50];
    let modes = gop.modes().map(Completion::unwrap);

    for m in modes
    {
        resolutions[resolutions_size] = m.info().resolution();
        resolutions_size += 1;

        if resolutions_size > 50 {
            break;
        }
    }

    let mode_info = gop.current_mode_info();
    let mut framebuffer = gop.frame_buffer();
    let slice = unsafe { slice::from_raw_parts_mut(framebuffer.as_mut_ptr(), framebuffer.size()) };
    let info = FrameBufferInfo {
        byte_len: framebuffer.size(),
        horizontal_resolution: mode_info.resolution().0,
        vertical_resolution: mode_info.resolution().1,
        pixel_format: match mode_info.pixel_format() {
            GopPixelFormat::Rgb => PixelFormat::RGB,
            GopPixelFormat::Bgr => PixelFormat::BGR,
            GopPixelFormat::Bitmask | GopPixelFormat::BltOnly => {
                panic!("Bitmask and BltOnly framebuffers are not supported")
            }
        },
        bytes_per_pixel: 4,
        stride: mode_info.stride(),
    };

    log::info!("UEFI boot");

    // Initialize a text-based logger using the given pixel-based framebuffer as output.
    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(slice, info));
    log::set_logger(logger).expect("logger already set");
    log::set_max_level(log::LevelFilter::Trace);
    log::info!("Framebuffer info: {:?}", info);

    log::info!("{} available graphic modes", resolutions_size);
    //log::info!("{:?}", resolutions);

    (PhysAddr::new(framebuffer.as_mut_ptr() as u64), info)
}

/// Contains the addresses of all memory mappings set up by [`set_up_mappings`].
pub struct Mappings {
    /// The entry point address of the kernel.
    pub entry_point: VirtAddr,
    /// The stack end page of the kernel.
    pub stack_end: Page,
    /// Keeps track of used entries in the level 4 page table, useful for finding a free
    /// virtual memory when needed.
    pub used_entries: UsedLevel4Entries,
    /// The start address of the framebuffer, if any.
    pub framebuffer: Option<VirtAddr>,
    /// The start address of the physical memory mapping, if enabled.
    pub physical_memory_offset: Option<VirtAddr>,
    /// The level 4 page table index of the recursive mapping, if enabled.
    pub recursive_index: Option<PageTableIndex>,
    /// The thread local storage template of the kernel executable, if it contains one.
    pub tls_template: Option<TlsTemplate>,
}

/// Memory addresses required for the context switch.
struct Addresses {
    page_table: PhysFrame,
    stack_top: VirtAddr,
    entry_point: VirtAddr,
    boot_info: &'static mut crate::boot_info::BootInfo,
}

/// Performs the actual context switch.
unsafe fn context_switch(addresses: Addresses) -> ! {
    asm!(
        "mov cr3, {}; mov rsp, {}; push 0; jmp {}",
        in(reg) addresses.page_table.start_address().as_u64(),
        in(reg) addresses.stack_top.as_u64(),
        in(reg) addresses.entry_point.as_u64(),
        in("rdi") addresses.boot_info as *const _ as usize,
    );

    unreachable!();
}

/// Creates page table abstraction types for both the bootloader and kernel page tables.
fn create_page_tables(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> PageTables {
    // UEFI identity-maps all memory, so the offset between physical and virtual addresses is 0
    let phys_offset = VirtAddr::new(0);

    // copy the currently active level 4 page table, because it might be read-only
    log::trace!("switching to new level 4 table");
    let bootloader_page_table = {
        let old_table = {
            let frame = x86_64::registers::control::Cr3::read().0;
            let ptr: *const PageTable = (phys_offset + frame.start_address().as_u64()).as_ptr();
            unsafe { &*ptr }
        };
        let new_frame = frame_allocator
            .allocate_frame()
            .expect("Failed to allocate frame for new level 4 table");
        let new_table: &mut PageTable = {
            let ptr: *mut PageTable =
                (phys_offset + new_frame.start_address().as_u64()).as_mut_ptr();
            // create a new, empty page table
            unsafe {
                ptr.write(PageTable::new());
                &mut *ptr
            }
        };

        // copy the first entry (we don't need to access more than 512 GiB; also, some UEFI
        // implementations seem to create an level 4 table entry 0 in all slots)
        new_table[0] = old_table[0].clone();

        // the first level 4 table entry is now identical, so we can just load the new one
        unsafe {
            x86_64::registers::control::Cr3::write(
                new_frame,
                x86_64::registers::control::Cr3Flags::empty(),
            );
            OffsetPageTable::new(&mut *new_table, phys_offset)
        }
    };

    // create a new page table hierarchy for the kernel
    let (kernel_page_table, kernel_level_4_frame) = {
        // get an unused frame for new level 4 page table
        let frame: PhysFrame = frame_allocator.allocate_frame().expect("no unused frames");
        log::info!("New page table at: {:#?}", &frame);
        // get the corresponding virtual address
        let addr = phys_offset + frame.start_address().as_u64();
        // initialize a new page table
        let ptr = addr.as_mut_ptr();
        unsafe { *ptr = PageTable::new() };
        let level_4_table = unsafe { &mut *ptr };
        (
            unsafe { OffsetPageTable::new(level_4_table, phys_offset) },
            frame,
        )
    };

    PageTables {
        bootloader: bootloader_page_table,
        kernel: kernel_page_table,
        kernel_level_4_frame,
    }
}

/// Sets up mappings for a kernel stack and the framebuffer.
///
/// The `kernel_bytes` slice should contain the raw bytes of the kernel ELF executable. The
/// `frame_allocator` argument should be created from the memory map. The `page_tables`
/// argument should point to the bootloader and kernel page tables. The function tries to parse
/// the ELF file and create all specified mappings in the kernel-level page table.
///
/// The `framebuffer_addr` and `framebuffer_size` fields should be set to the start address and
/// byte length the pixel-based framebuffer. These arguments are required because the functions
/// maps this framebuffer in the kernel-level page table, unless the `map_framebuffer` config
/// option is disabled.
///
/// This function reacts to unexpected situations (e.g. invalid kernel ELF file) with a panic, so
/// errors are not recoverable.
pub fn set_up_mappings<I, D>(
    kernel_bytes: &[u8],
    frame_allocator: &mut LegacyFrameAllocator<I, D>,
    page_tables: &mut PageTables,
    framebuffer_addr: PhysAddr,
    framebuffer_size: usize,
) -> Mappings
    where
        I: ExactSizeIterator<Item = D> + Clone,
        D: LegacyMemoryRegion,
{
    let kernel_page_table = &mut page_tables.kernel;

    // Enable support for the no-execute bit in page tables.
    enable_nxe_bit();
    // Make the kernel respect the write-protection bits even when in ring 0 by default
    enable_write_protect_bit();

    let (entry_point, tls_template, mut used_entries) =
        load_kernel::load_kernel(kernel_bytes, kernel_page_table, frame_allocator)
            .expect("no entry point");
    log::info!("Entry point at: {:#x}", entry_point.as_u64());

    // create a stack
    let stack_start_addr = used_entries.get_free_address();
    let stack_start: Page = Page::containing_address(stack_start_addr);
    let stack_end = {
        let end_addr = stack_start_addr + 20 * PAGE_SIZE;
        Page::containing_address(end_addr - 1u64)
    };
    for page in Page::range_inclusive(stack_start, stack_end) {
        let frame = frame_allocator
            .allocate_frame()
            .expect("frame allocation failed when mapping a kernel stack");
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        match unsafe { kernel_page_table.map_to(page, frame, flags, frame_allocator) } {
            Ok(tlb) => tlb.flush(),
            Err(err) => panic!("failed to map page {:?}: {:?}", page, err),
        }
    }

    // identity-map context switch function, so that we don't get an immediate pagefault
    // after switching the active page table
    let context_switch_function = PhysAddr::new(context_switch as *const () as u64);
    let context_switch_function_start_frame: PhysFrame =
        PhysFrame::containing_address(context_switch_function);
    for frame in PhysFrame::range_inclusive(
        context_switch_function_start_frame,
        context_switch_function_start_frame + 1,
    ) {
        match unsafe {
            kernel_page_table.identity_map(frame, PageTableFlags::PRESENT, frame_allocator)
        } {
            Ok(tlb) => tlb.flush(),
            Err(err) => panic!("failed to identity map frame {:?}: {:?}", frame, err),
        }
    }

    // create, load, and identity-map GDT (required for working `iretq`)
    let gdt_frame = frame_allocator
        .allocate_frame()
        .expect("failed to allocate GDT frame");
    create_and_load_gdt(gdt_frame);
    match unsafe {
        kernel_page_table.identity_map(gdt_frame, PageTableFlags::PRESENT, frame_allocator)
    } {
        Ok(tlb) => tlb.flush(),
        Err(err) => panic!("failed to identity map frame {:?}: {:?}", gdt_frame, err),
    }

    // map framebuffer
    let framebuffer_virt_addr = if true {
        log::info!("Map framebuffer");

        let framebuffer_start_frame: PhysFrame = PhysFrame::containing_address(framebuffer_addr);
        let framebuffer_end_frame =
            PhysFrame::containing_address(framebuffer_addr + framebuffer_size - 1u64);
        let start_page = Page::containing_address(used_entries.get_free_address());
        for (i, frame) in
        PhysFrame::range_inclusive(framebuffer_start_frame, framebuffer_end_frame).enumerate()
        {
            let page = start_page + u64::try_from(i).expect("Numeric overflow");
            let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
            match unsafe { kernel_page_table.map_to(page, frame, flags, frame_allocator) } {
                Ok(tlb) => tlb.flush(),
                Err(err) => panic!(
                    "failed to map page {:?} to frame {:?}: {:?}",
                    page, frame, err
                ),
            }
        }
        let framebuffer_virt_addr = start_page.start_address();
        Some(framebuffer_virt_addr)
    } else {
        None
    };

    let physical_memory_offset = if true {
        log::info!("Map physical memory");
        let offset = used_entries.get_free_address();

        let start_frame = PhysFrame::containing_address(PhysAddr::new(0));
        let max_phys = frame_allocator.max_phys_addr();
        let end_frame: PhysFrame<Size2MiB> = PhysFrame::containing_address(PhysAddr::new((max_phys - 1u64).0));
        for frame in PhysFrame::range_inclusive(start_frame, end_frame) {
            let page = Page::containing_address(offset + frame.start_address().as_u64());
            let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
            match unsafe { kernel_page_table.map_to(page, frame, flags, frame_allocator) } {
                Ok(tlb) => tlb.ignore(),
                Err(err) => panic!(
                    "failed to map page {:?} to frame {:?}: {:?}",
                    page, frame, err
                ),
            };
        }

        Some(offset)
    } else {
        None
    };

    let recursive_index = None; /*if CONFIG.map_page_table_recursively {
        log::info!("Map page table recursively");
        let index = CONFIG
            .recursive_index
            .map(PageTableIndex::new)
            .unwrap_or_else(|| used_entries.get_free_entry());

        let entry = &mut kernel_page_table.level_4_table()[index];
        if !entry.is_unused() {
            panic!(
                "Could not set up recursive mapping: index {} already in use",
                u16::from(index)
            );
        }
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        entry.set_frame(page_tables.kernel_level_4_frame, flags);

        Some(index)
    } else {
        None
    };*/

    Mappings {
        framebuffer: framebuffer_virt_addr,
        entry_point,
        stack_end,
        used_entries,
        physical_memory_offset,
        recursive_index,
        tls_template,
    }
}

pub fn create_and_load_gdt(frame: PhysFrame) {
    let phys_addr = frame.start_address();
    log::info!("Creating GDT at {:?}", phys_addr);
    let virt_addr = VirtAddr::new(phys_addr.as_u64()); // utilize identity mapping

    let ptr: *mut GlobalDescriptorTable = virt_addr.as_mut_ptr();

    let mut gdt = GlobalDescriptorTable::new();
    let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
    let data_selector = gdt.add_entry(Descriptor::kernel_data_segment());
    let gdt = unsafe {
        ptr.write(gdt);
        &*ptr
    };

    gdt.load();
    unsafe {
        CS::set_reg(code_selector);
        DS::set_reg(data_selector);
        ES::set_reg(data_selector);
        SS::set_reg(data_selector);
    }
}

/// Allocates and initializes the boot info struct and the memory map.
///
/// The boot info and memory map are mapped to both the kernel and bootloader
/// address space at the same address. This makes it possible to return a Rust
/// reference that is valid in both address spaces. The necessary physical frames
/// are taken from the given `frame_allocator`.
pub fn create_boot_info<I, D>(
    mut frame_allocator: LegacyFrameAllocator<I, D>,
    page_tables: &mut PageTables,
    mappings: &mut Mappings,
    system_info: SystemInfo,
) -> &'static mut crate::boot_info::BootInfo
    where
        I: ExactSizeIterator<Item = D> + Clone,
        D: LegacyMemoryRegion,
{
    log::info!("Allocate bootinfo");

    // allocate and map space for the boot info
    let (boot_info, memory_regions) = {
        let boot_info_addr = mappings.used_entries.get_free_address();
        let boot_info_end = boot_info_addr + mem::size_of::<BootInfo>();
        let memory_map_regions_addr =
            boot_info_end.align_up(u64::try_from(mem::align_of::<MemoryRegion>()).expect("Numeric fault"));
        let regions = frame_allocator.len() + 1; // one region might be split into used/unused
        let memory_map_regions_end =
            memory_map_regions_addr + regions * mem::size_of::<MemoryRegion>();

        let start_page = Page::containing_address(boot_info_addr);
        let end_page = Page::containing_address(memory_map_regions_end - 1u64);
        for page in Page::range_inclusive(start_page, end_page) {
            let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
            let frame = frame_allocator
                .allocate_frame()
                .expect("frame allocation for boot info failed");
            match unsafe {
                page_tables
                    .kernel
                    .map_to(page, frame, flags, &mut frame_allocator)
            } {
                Ok(tlb) => tlb.flush(),
                Err(err) => panic!("failed to map page {:?}: {:?}", page, err),
            }
            // we need to be able to access it too
            match unsafe {
                page_tables
                    .bootloader
                    .map_to(page, frame, flags, &mut frame_allocator)
            } {
                Ok(tlb) => tlb.flush(),
                Err(err) => panic!("failed to map page {:?}: {:?}", page, err),
            }
        }

        let boot_info: &'static mut MaybeUninit<crate::boot_info::BootInfo> =
            unsafe { &mut *boot_info_addr.as_mut_ptr() };
        let memory_regions: &'static mut [MaybeUninit<MemoryRegion>] =
            unsafe { slice::from_raw_parts_mut(memory_map_regions_addr.as_mut_ptr(), regions) };
        (boot_info, memory_regions)
    };

    log::info!("Create Memory Map");

    // build memory map
    let memory_regions = frame_allocator.construct_memory_map(memory_regions);

    log::info!("Create bootinfo");

    let mut b = crate::boot_info::BootInfo::new(memory_regions.into());
    b.version_major = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap();
    b.version_minor = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap();
    b.version_patch = env!("CARGO_PKG_VERSION_PATCH").parse().unwrap();
    b.pre_release = !env!("CARGO_PKG_VERSION_PRE").is_empty();

    let fb = mappings
        .framebuffer
        .map(|addr| FrameBuffer {
            buffer_start: addr.as_u64(),
            buffer_byte_len: system_info.framebuffer_info.byte_len,
            info: system_info.framebuffer_info,
        });

    b.framebuffer = match fb {
        None => Optional::None,
        Some(f) => Optional::Some(f)
    };

    b.physical_memory_offset = mappings.physical_memory_offset.map(VirtAddr::as_u64).into();
    b.recursive_index = mappings.recursive_index.map(Into::into).into();
    b.rsdp_addr = system_info.rsdp_addr.map(|addr| addr.as_u64()).into();
    b.tls_template = mappings.tls_template.into();

    let boot_info = boot_info.write(b);

    boot_info
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        logger::LOGGER
            .get()
            .map(|l| l.force_unlock())
    };
    log::error!("{}", info);
    loop {
        unsafe { asm!("cli; hlt") };
    }
}

#[alloc_error_handler]
fn alloc_error_handler(_: core::alloc::Layout) -> ! {
    panic!("Allocation error");
}
