#![feature(ptr_metadata)]
#![no_std]
#![no_main]

extern crate shared_lib;
extern crate alloc;

use core::{
    panic::PanicInfo,
    arch::asm,
    ptr::addr_of,
    slice::{
        from_raw_parts_mut,
        from_raw_parts
    }
};
use uefi::prelude::entry;
use uefi::proto::media::file::File;
use uefi::table::boot::{OpenProtocolAttributes, OpenProtocolParams, AllocateType, MemoryType};
use uefi::proto::media::{
    file::{FileMode, FileAttribute, RegularFile},
    fs::SimpleFileSystem
};
use uefi::data_types::CStr16;
use uefi::proto::console::gop::GraphicsOutput;
use xmas_elf::{ElfFile, header, program};
use shared_lib::addr::{PhysAddr, VirtAddr};
use shared_lib::logger::FrameBufferInfo;
use shared_lib::page_table::{PageTable, PageTablesAllocator, map_address, remap_address, align_down, align_down_u64};
use shared_lib::{BootInfo, logger, VIRT_MAPPING_OFFSET};
use shared_lib::allocator::ALLOCATOR;
use shared_lib::frame_allocator::{MemoryRegion, FrameAllocator, MemoryMap, MAX_MEMORY_MAP_SIZE, MEMORY_MAP_PAGES};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe {
        logger::LOGGER
            .get()
            .map(|l| l.force_unlock())
    };

    log::info!("{}", info);

    loop {
        unsafe { asm!("cli; hlt") };
    }
}

fn convert_memory_type(t: MemoryType) -> shared_lib::frame_allocator::MemoryType {
    match t {
        MemoryType::MMIO_PORT_SPACE | MemoryType::MMIO
        | MemoryType::RESERVED | MemoryType::UNUSABLE => shared_lib::frame_allocator::MemoryType::Reserved,

        MemoryType::PERSISTENT_MEMORY | MemoryType::CONVENTIONAL
        | MemoryType::LOADER_DATA | MemoryType::LOADER_CODE
        | MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => shared_lib::frame_allocator::MemoryType::Free,

        MemoryType::ACPI_NON_VOLATILE | MemoryType::RUNTIME_SERVICES_CODE
        | MemoryType::RUNTIME_SERVICES_DATA => shared_lib::frame_allocator::MemoryType::Acpi1_3,

        MemoryType::ACPI_RECLAIM => shared_lib::frame_allocator::MemoryType::AcpiReclaim,

        MemoryType::PAL_CODE => shared_lib::frame_allocator::MemoryType::Acpi1_4,

        _ => panic!("Unexpected memory type")
    }
}

fn init_framebuffer(image: uefi::Handle, system_table: &mut uefi::table::SystemTable<uefi::table::Boot>)
    -> Result<FrameBufferInfo, &'static str> {
    let gop_handle = system_table
        .boot_services()
        .get_handle_for_protocol::<GraphicsOutput>()
        .expect("Failed to get GOP handle");

    let mut gop = unsafe {
        system_table.boot_services()
            .open_protocol::<GraphicsOutput>(
                OpenProtocolParams {
                    handle: gop_handle,
                    agent: image,
                    controller: None
                },
                OpenProtocolAttributes::Exclusive,
            )
            .expect("Failed to open GOP protocol")
    };

    let mode_info = gop.current_mode_info();

    Ok(FrameBufferInfo {
        addr: gop.frame_buffer().as_mut_ptr() as u64,
        size: gop.frame_buffer().size(),
        width: mode_info.resolution().0,
        height: mode_info.resolution().1,
        pixel_format: match mode_info.pixel_format() {
            uefi::proto::console::gop::PixelFormat::Rgb => shared_lib::logger::PixelFormat::Rgb,
            uefi::proto::console::gop::PixelFormat::Bgr => shared_lib::logger::PixelFormat::Bgr,
            uefi::proto::console::gop::PixelFormat::Bitmask => shared_lib::logger::PixelFormat::Bitmask,
            uefi::proto::console::gop::PixelFormat::BltOnly => shared_lib::logger::PixelFormat::BltOnly
        },
        stride: mode_info.stride()
    })
}

fn load_kernel(image: uefi::Handle, system_table: &mut uefi::table::SystemTable<uefi::table::Boot>, kernel_max_size: usize)
    -> Result<*const u8, &'static str> {
    let pages_count = 1 + kernel_max_size / 4096;

    let fs_handle = system_table
        .boot_services()
        .get_handle_for_protocol::<SimpleFileSystem>()
        .expect("Failed to get FS handle");

    let mut fs = unsafe {
        system_table.boot_services()
            .open_protocol::<SimpleFileSystem>(
                OpenProtocolParams {
                    handle: fs_handle,
                    agent: image,
                    controller: None
                },
                OpenProtocolAttributes::Exclusive,
            )
            .expect("Failed to open GOP protocol")
    };

    let mut root_fs = fs.open_volume().expect("Failed to open volume");

    let mut buff: [u16; 16] = [0; 16];
    let kernel_name = CStr16::from_str_with_buf("kernel", &mut buff)
        .expect("Failed to create CStr16");
    let handle = root_fs.open(kernel_name, FileMode::Read, FileAttribute::READ_ONLY)
        .expect("Failed to open kernel file");

    let mut file = unsafe { RegularFile::new(handle) };

    let kernel = {
        let ptr = system_table
            .boot_services()
            .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages_count)
            .expect("Failed to allocate page for kernel");
        unsafe { from_raw_parts_mut(ptr as *mut u8, kernel_max_size) }
    };

    file.read(kernel)
        .expect("Failed to read kernel file");

    Ok(kernel.as_ptr())
}

unsafe fn init_allocator(memory_map: uefi::table::boot::MemoryMap)
                         -> Result<(FrameAllocator, MemoryMap), &'static str> {
    static mut MMAP: MemoryMap = MemoryMap {
        entries: [ MemoryRegion{ ty: shared_lib::frame_allocator::MemoryType::Reserved, addr: 0, page_count: 0 }; MAX_MEMORY_MAP_SIZE ],
        next_free_entry_idx: 0
    };

    for (idx, memory_descriptor) in memory_map.entries().enumerate() {
        if memory_descriptor.phys_start == 0 {
            MMAP.entries[idx] = MemoryRegion {
                ty: shared_lib::frame_allocator::MemoryType::Reserved,
                addr: memory_descriptor.phys_start,
                page_count: memory_descriptor.page_count as usize
            };
            continue;
        }

        MMAP.entries[idx] = MemoryRegion {
            ty: convert_memory_type(memory_descriptor.ty),
            addr: memory_descriptor.phys_start,
            page_count: memory_descriptor.page_count as usize
        };
    }
    MMAP.next_free_entry_idx = (memory_map.entries().len()) as u64;

    Ok((FrameAllocator::new(addr_of!(MMAP), 0, 0), MMAP.clone()))
}

#[derive(Copy, Clone)]
struct MappedEntry {
    pub page: VirtAddr,
    pub frame: u64
}

fn map_kernel(elf_file: &ElfFile, kernel: u64, page_table: &mut PageTable, allocator: &mut FrameAllocator) -> Result<(), &'static str> {
    let mut mapped_frames: [MappedEntry; 100] = [ MappedEntry{ page: VirtAddr::zero(), frame: 0 }; 100 ];
    let mut mapped_frames_counter = 0;

    for header in elf_file.program_iter() {
        match header.get_type().unwrap() {
            program::Type::Load => {
                let phys_start_addr = (kernel as u64) + header.offset();
                let phys_end_addr = phys_start_addr + header.file_size();

                let virt_start_addr = VirtAddr::new_checked(header.virtual_addr())
                    .expect("Got bad virtual address from ELF");

                log::debug!("[kernel map] segment: {}, phys_start: {:#x}, phys_end: {:#x}. header file size: {}",
                    virt_start_addr, phys_start_addr, phys_end_addr, header.file_size());

                if header.file_size() != 0 {
                    let virt_start_addr_aligned = align_down(virt_start_addr);
                    let phys_start_addr_aligned = align_down_u64(phys_start_addr);

                    for i in 0..(1 + (header.file_size() - 1 + virt_start_addr.0 - virt_start_addr_aligned.0) / 4096) {
                        let virt = virt_start_addr_aligned.offset(i * 4096).unwrap();
                        let phys = phys_start_addr_aligned + i * 4096;

                        log::debug!("[kernel map] Mapping {} to {:#x}", virt, phys);
                        unsafe {
                            map_address(page_table, virt, phys, allocator)
                                .expect("Failed to map kernel");
                        }
                        mapped_frames[mapped_frames_counter] = MappedEntry { page: virt, frame: phys };
                        mapped_frames_counter += 1;
                    }
                } else {
                    let virt_start_addr_aligned = align_down(virt_start_addr);
                    let phys_start_addr_aligned = align_down_u64(phys_start_addr);

                    log::debug!("[kernel map] Mapping {} to {:#x}", virt_start_addr_aligned, phys_start_addr_aligned);
                    unsafe {
                        map_address(page_table, virt_start_addr_aligned, phys_start_addr_aligned, allocator)
                            .expect("Failed to map kernel");
                    }
                    mapped_frames[mapped_frames_counter] = MappedEntry { page: virt_start_addr_aligned, frame: phys_start_addr_aligned };
                    mapped_frames_counter += 1;
                }

                if header.mem_size() > header.file_size() {
                    let zero_start = virt_start_addr.offset(header.file_size()).unwrap();
                    let zero_end = virt_start_addr.offset(header.mem_size()).unwrap();

                    log::debug!("[kernel map] .bss section: from {} to {}. size: {}", zero_start, zero_end, header.mem_size() - header.file_size());

                    let mut data_bytes_before_zero = zero_start.0 & 0xfff;

                    if data_bytes_before_zero != 0 {
                        let frame = allocator.allocate_frame().expect("Failed to allocate new frame");
                        unsafe {
                            let frame_to_copy = align_down_u64(phys_end_addr);
                            for i in 0..mapped_frames_counter {
                                if mapped_frames[i].frame == frame_to_copy {
                                    log::debug!("[kernel map] Remapping {} to {:#x}", mapped_frames[i].page, frame);
                                    remap_address(page_table, mapped_frames[i].page, frame, allocator)
                                        .expect("Failed to map kernel");
                                }
                            }

                            log::debug!("[kernel map] Copying from {:#x}", align_down_u64(phys_end_addr));
                            core::ptr::copy(
                                align_down_u64(phys_end_addr) as *const u8,
                                frame as *mut _,
                                data_bytes_before_zero as usize,
                            );

                            core::ptr::write_bytes(
                                (frame + data_bytes_before_zero) as *mut u8,
                                0,
                                (4096 - data_bytes_before_zero) as usize,
                            );
                        }
                    } else {
                        data_bytes_before_zero = 4096;
                    }

                    if header.mem_size() - header.file_size() > (4096 - data_bytes_before_zero) {
                        let zero_start_aligned = zero_start.offset(4096 - data_bytes_before_zero).unwrap();
                        let bytes_to_allocate = header.mem_size() - header.file_size() - (4096 - data_bytes_before_zero);
                        log::debug!("[kernel map] bytes_to_allocate: {}", bytes_to_allocate);

                        for i in 0..(1 + bytes_to_allocate / 4096) {
                            let frame = allocator.allocate_frame().expect("Failed to allocate new frame");
                            let virt_ptr = zero_start_aligned.offset(i * 4096).unwrap();
                            log::debug!("[kernel map] Mapping {} to {:#x}", virt_ptr, frame);

                            unsafe {
                                map_address(page_table, virt_ptr, frame, allocator)
                                    .expect("Failed to map kernel");
                                core::ptr::write_bytes(
                                    frame as *mut u8,
                                    0,
                                    4096,
                                );
                            }
                        }
                    }
                }
            }
            program::Type::Tls => { unimplemented!("Not implemented TLS section") }
            _ => {}
        }
    }
    Ok(())
}

fn map_framebuffer(framebuffer: &FrameBufferInfo, page_table: &mut PageTable, allocator: &mut FrameAllocator) -> Result<(), &'static str> {
    let fb_start = framebuffer.addr;
    let fb_end = framebuffer.addr + framebuffer.size as u64 - 1;
    let pages_needed_for_fb = framebuffer.size / 4096;
    log::info!("Mapping framebuffer. addr: {:#x} - {:#x}. Pages for framebuffer: {}", fb_start, fb_end, pages_needed_for_fb);

    for i in 0..pages_needed_for_fb {
        let ptr = fb_start + i as u64 * 4096;
        unsafe {
            map_address(page_table, VirtAddr::new_checked(ptr + VIRT_MAPPING_OFFSET).unwrap(), ptr, allocator)
                .expect("Failed to map framebuffer");
        }
    }

    Ok(())
}

fn create_stack(stack_addr: PhysAddr, stack_depth: usize, page_table: &mut PageTable, allocator: &mut FrameAllocator) -> Result<u64, &'static str> {
    log::info!("Mapping stack");
    for i in 0..stack_depth {
        let ptr = stack_addr.0 + i as u64 * 4096;
        unsafe {
            map_address(page_table, VirtAddr::new_checked(ptr).unwrap(), ptr, allocator)
                .expect("Failed to map stack");
        }
    }
    Ok(stack_addr.0 + (stack_depth as u64 - 1) * 4096)
}

fn setup_mappings(last_frame_addr: PhysAddr, page_table: &mut PageTable, allocator: &mut FrameAllocator, kernel: *const u8, kernel_size: usize, framebuffer: &FrameBufferInfo) -> VirtAddr {
    let elf_file = ElfFile::new(unsafe { from_raw_parts(kernel, kernel_size) }).unwrap();
    header::sanity_check(&elf_file).expect("Failed to parse kernel file. Expected ELF");

    log::info!("Mapping all memory. Last frame: {:#x}", last_frame_addr.0);

    for i in 0..(last_frame_addr.0 / 4096) {
        let phys =  i * 4096;
        let virt = VirtAddr::new(phys + VIRT_MAPPING_OFFSET);

        unsafe {
            map_address(page_table, virt, phys, allocator)
                .expect("Failed to map memory");
        }
    }

    map_kernel(&elf_file, kernel as u64, page_table, allocator)
        .expect("Failed to map kernel");

    map_framebuffer(&framebuffer, page_table, allocator)
        .expect("Failed to map framebuffer");

    unsafe {
        let ctx_switch_ptr = context_switch as *const () as u64;
        map_address(page_table, align_down(VirtAddr::new_checked(ctx_switch_ptr).unwrap()), align_down_u64(ctx_switch_ptr), allocator)
            .expect("Failed to map context switch function");
    }

    VirtAddr::new_checked(elf_file.header.pt2.entry_point()).unwrap()
}

fn init_logger(image: uefi::Handle, system_table: &mut uefi::table::SystemTable<uefi::table::Boot>) -> FrameBufferInfo {
    let framebuffer = init_framebuffer(image, system_table)
        .expect("Failed to init framebuffer");

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(framebuffer));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    framebuffer
}

fn map_bootinfo(boot_info: &BootInfo, page_table: &mut PageTable, allocator: &mut FrameAllocator) {
    let boot_info_ptr = boot_info as *const _ as u64;
    log::info!("Mapping boot info. addr: {:#x}", boot_info_ptr);

    unsafe {
        map_address(page_table, align_down(VirtAddr::new_checked(boot_info_ptr).unwrap()), align_down_u64(boot_info_ptr), allocator)
            .expect("Failed to map boot info");
    }

    for i in 0..=MEMORY_MAP_PAGES {
        let ptr = align_down_u64(boot_info.memory_map.entries.as_ptr() as u64) + i as u64 * 4096;
        unsafe {
            map_address(page_table, VirtAddr::new_checked(ptr).unwrap(), ptr, allocator)
                .expect("Failed to map boot info");
        }
    }
}

fn setup_heap(system_table: &mut uefi::table::SystemTable<uefi::table::Boot>) {
    let heap_size_in_pages = 25;
    let heap_addr = PhysAddr(u64::from(system_table
        .boot_services()
        .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, heap_size_in_pages)
        .unwrap()));

    unsafe {
        ALLOCATOR.lock().init(heap_addr.0 as usize, heap_size_in_pages * 4096);
    }
}

#[entry]
fn efi_main(image: uefi::Handle, mut system_table: uefi::table::SystemTable<uefi::table::Boot>) -> uefi::Status {
    setup_heap(&mut system_table);
    let mut framebuffer = init_logger(image, &mut system_table);

    log::info!("This is a very simple UEFI bootloader");

    let kernel_max_size = 100 * 4096;
    let kernel = load_kernel(image, &mut system_table, kernel_max_size)
        .expect("Failed to load kernel");

    let stack_depth = 20;
    let stack_addr = PhysAddr(u64::from(system_table
        .boot_services()
        .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, stack_depth)
        .unwrap()));

    log::info!("Exiting boot services...");
    let (runtime_system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);

    let last_memory_region = memory_map.entries().last().unwrap();
    let last_frame_addr = last_memory_region.phys_start + (last_memory_region.page_count - 1) * 4096;

    let (mut allocator, memory_map) = unsafe {
        init_allocator(memory_map)
            .expect("Failed to create Allocator")
    };

    // convert to and from raw ptr to bypass borrow checker
    let page_table = unsafe {
        let page_table_ptr = allocator.allocate_page_table()
            .expect("Failed to allocate page table")
            as *mut PageTable;

        &mut *page_table_ptr
    };

    let entry_point = setup_mappings(PhysAddr(u64::from(last_frame_addr)), page_table, &mut allocator, kernel, kernel_max_size, &framebuffer);

    framebuffer.addr += VIRT_MAPPING_OFFSET;

    let stack = create_stack(stack_addr, 20, page_table, &mut allocator)
        .expect("Failed to create stack");

    let rsdp_addr = {
        use uefi::table::cfg;
        let mut config_entries = runtime_system_table.config_table().iter();
        // look for an ACPI2 RSDP first
        let acpi2_rsdp = config_entries.find(|entry| matches!(entry.guid, cfg::ACPI2_GUID));
        if acpi2_rsdp.is_some() {
            log::info!("ACPI2 found! {:#x}", acpi2_rsdp.unwrap().address as u64);
        }

        // if no ACPI2 RSDP is found, look for a ACPI1 RSDP
        let rsdp = acpi2_rsdp
            .or_else(|| config_entries.find(|entry| matches!(entry.guid, cfg::ACPI_GUID)));
        rsdp.map(|entry| entry.address as u64)
    };

    log::info!("Page table: {:#x}", page_table as *const PageTable as u64);
    log::info!("rsp: {:#x}", stack);
    log::info!("Jumping to kernel entry point at {:#x}", entry_point.0);
    log::info!("Kernel address: {:#x}", kernel as u64);
    log::info!("FB addr: {:#x}", framebuffer.addr);
    log::info!("FB info: {:#x}", &framebuffer as *const _ as u64);
    log::info!("RSDP: {:#x}", rsdp_addr.unwrap_or(0));

    let mut boot_info = BootInfo{ fb_info: framebuffer, rsdp_addr: rsdp_addr.unwrap_or(0), memory_map, memory_map_next_free_frame: 0 };

    map_bootinfo(&boot_info, page_table, &mut allocator);

    boot_info.memory_map_next_free_frame = allocator.next;

    unsafe {
        context_switch(page_table as *const PageTable as u64, entry_point.0, stack, &boot_info);
    }
}

unsafe fn context_switch(page_table: u64, entry_point: u64, stack_top: u64, boot_info: &BootInfo) -> ! {
    asm!(
    "mov cr3, {}; mov rsp, {}; push 0; jmp {}",
    in(reg) page_table,
    in(reg) stack_top,
    in(reg) entry_point,
    in("rdi") boot_info as *const _ as u64,
    );

    unreachable!();
}
