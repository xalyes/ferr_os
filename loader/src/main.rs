#![feature(ptr_metadata)]
#![no_std]
#![no_main]

extern crate shared_lib;

use core::panic::PanicInfo;
use core::arch::asm;
use core::slice::{ from_raw_parts_mut, from_raw_parts };

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
use shared_lib::logger::FrameBufferInfo;
use shared_lib::{ logger, PageTable, PageTablesAllocator, map_address, remap_address, align_down };
use shared_lib::allocator::{ MemoryRegion, Allocator };

fn convert_memory_type(t: MemoryType) -> shared_lib::allocator::MemoryType {
    match t {
        MemoryType::MMIO_PORT_SPACE | MemoryType::MMIO
        | MemoryType::RESERVED | MemoryType::UNUSABLE => shared_lib::allocator::MemoryType::Reserved,

        MemoryType::PERSISTENT_MEMORY | MemoryType::CONVENTIONAL
        | MemoryType::LOADER_DATA | MemoryType::LOADER_CODE
        | MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => shared_lib::allocator::MemoryType::Free,

        MemoryType::ACPI_NON_VOLATILE | MemoryType::RUNTIME_SERVICES_CODE
        | MemoryType::RUNTIME_SERVICES_DATA => shared_lib::allocator::MemoryType::Acpi1_3,

        MemoryType::ACPI_RECLAIM => shared_lib::allocator::MemoryType::AcpiReclaim,

        MemoryType::PAL_CODE => shared_lib::allocator::MemoryType::Acpi1_4,

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

fn init_allocator(memory_map: uefi::table::boot::MemoryMap)
    -> Result<shared_lib::allocator::Allocator, &'static str> {

    let memory_map_size = core::mem::size_of::<shared_lib::allocator::MemoryRegion>() * memory_map.entries().count();
    let pages_to_find = 1 + (memory_map_size / 4096);
    log::info!("Finding region for loader's memory map... Needed size: {} - {} pages", memory_map_size, pages_to_find);

    let mut find_result: Option<(u64, usize)> = None;
    for memory_descriptor in memory_map.entries() {
        if memory_descriptor.page_count >= pages_to_find as u64
            && matches!(convert_memory_type(memory_descriptor.ty), shared_lib::allocator::MemoryType::Free)
            && memory_descriptor.phys_start != 0 {
            find_result = Option::from((memory_descriptor.phys_start, memory_descriptor.page_count as usize));
            break;
        }
    }
    let memory_region_for_memory_map = find_result
        .expect("Failed to find memory region for memory map");

    let loader_memory_map = unsafe { from_raw_parts_mut(memory_region_for_memory_map.0 as *mut MemoryRegion, pages_to_find * 4096) };

    log::info!("Found suitable memory region: addr: {:#x} pages: {}", find_result.unwrap().0, find_result.unwrap().1);

    for (idx, memory_descriptor) in memory_map.entries().enumerate() {
        if memory_descriptor.phys_start == find_result.unwrap().0 {
            if pages_to_find < memory_descriptor.page_count as usize {
                loader_memory_map[idx] = shared_lib::allocator::MemoryRegion {
                    ty: shared_lib::allocator::MemoryType::Free,
                    addr: memory_descriptor.phys_start + (4096 * pages_to_find) as u64,
                    page_count: memory_descriptor.page_count as usize - pages_to_find
                };
                continue;
            }
        }

        if memory_descriptor.phys_start == 0 {
            loader_memory_map[idx] = shared_lib::allocator::MemoryRegion {
                ty: shared_lib::allocator::MemoryType::Reserved,
                addr: memory_descriptor.phys_start,
                page_count: memory_descriptor.page_count as usize
            };
            continue;
        }

        loader_memory_map[idx] = shared_lib::allocator::MemoryRegion {
            ty: convert_memory_type(memory_descriptor.ty),
            addr: memory_descriptor.phys_start,
            page_count: memory_descriptor.page_count as usize
        };
    }

    loader_memory_map[memory_map.entries().len()] = shared_lib::allocator::MemoryRegion {
        ty: shared_lib::allocator::MemoryType::InUse,
        addr: find_result.unwrap().0,
        page_count: pages_to_find
    };

    for i in 0..10 {
        log::info!("Loader memory map entry addr: {:#x}, page_count: {}, type: {:?}",
            loader_memory_map[i].addr, loader_memory_map[i].page_count, loader_memory_map[i].ty);
    }

    log::info!("Loader memory map entry addr: {:#x}, page_count: {}, type: {:?}",
            loader_memory_map[memory_map.entries().len()].addr, loader_memory_map[memory_map.entries().len()].page_count, loader_memory_map[memory_map.entries().len()].ty);

    Ok(shared_lib::allocator::Allocator::new(loader_memory_map, memory_map.entries().len() + 1))
}

fn map_kernel(elf_file: &ElfFile, kernel: u64, page_table: &mut PageTable, allocator: &mut Allocator) -> Result<(), &'static str> {
    for header in elf_file.program_iter() {
        match header.get_type().unwrap() {
            program::Type::Load => {
                let phys_start_addr = (kernel as u64) + header.offset();
                let phys_end_addr = phys_start_addr + header.file_size() - 1;

                let virt_start_addr = header.virtual_addr();

                log::info!("Handling segment phys: {:#x}, virt: {:#x}, phys_start: {:#x}, phys_end: {:#x}",
                    header.physical_addr(), virt_start_addr, phys_start_addr, phys_end_addr);

                let virt_start_addr_aligned = align_down(virt_start_addr);
                let phys_start_addr_aligned = align_down(phys_start_addr);

                for i in 0..(1 + (header.file_size() + virt_start_addr - virt_start_addr_aligned) / 4096) {
                    let virt = virt_start_addr_aligned + i * 4096;
                    let phys = phys_start_addr_aligned + i * 4096;

                    log::info!("Mapping {:#x} to {:#x}", virt, phys);
                    unsafe {
                        map_address(page_table, virt, phys, allocator)
                            .expect("Failed to map kernel");
                    }
                }

                if header.mem_size() > header.file_size() {
                    let zero_start = virt_start_addr + header.file_size();
                    let zero_end = virt_start_addr + header.mem_size();

                    log::info!(".bss section: from {:#x} to {:#x}. size: {}", zero_start, zero_end, header.mem_size() - header.file_size());

                    let data_bytes_before_zero = zero_start & 0xfff;

                    if data_bytes_before_zero != 0 {
                        let frame = allocator.allocate(1).expect("Failed to allocate new frame");
                        log::info!("Remapping {:#x} to {:#x}", align_down(zero_start - 1), frame);
                        unsafe {
                            remap_address(page_table, align_down(zero_start - 1), frame, allocator)
                                .expect("Failed to map kernel");

                            log::info!("Copying from {:#x}", align_down(phys_end_addr));
                            core::ptr::copy(
                                align_down(phys_end_addr) as *const u8,
                                frame as *mut _,
                                data_bytes_before_zero as usize,
                            );

                            core::ptr::write_bytes(
                                zero_start as *mut u8,
                                0,
                                (4096 - data_bytes_before_zero) as usize,
                            );
                        }
                    }

                    if header.mem_size() - header.file_size() > 4096 {
                        unimplemented!(".bss section is over 4096! Implement this");
                    }
                }
            }
            program::Type::Tls => { unimplemented!("Not implemented TLS section") }
            _ => {}
        }
    }
    Ok(())
}

fn map_framebuffer(framebuffer: &FrameBufferInfo, page_table: &mut PageTable, allocator: &mut Allocator) -> Result<(), &'static str> {
    let fb_start = framebuffer.addr;
    let fb_end = framebuffer.addr + framebuffer.size as u64 - 1;
    let pages_needed_for_fb = framebuffer.size / 4096;
    log::info!("Mapping framebuffer. addr: {:#x} - {:#x}. Pages for framebuffer: {}", fb_start, fb_end, pages_needed_for_fb);

    for i in 0..pages_needed_for_fb {
        let ptr = fb_start + i as u64 * 4096;
        unsafe {
            map_address(page_table, ptr, ptr, allocator)
                .expect("Failed to map framebuffer");
        }
    }

    let fb_info_ptr = framebuffer as *const _ as u64;
    log::info!("Mapping framebuffer info. addr: {:#x}", fb_info_ptr);
    unsafe {
        map_address(page_table, align_down(fb_info_ptr), align_down(fb_info_ptr), allocator)
            .expect("Failed to map fb info");
    }
    Ok(())
}

fn create_stack(stack_depth: usize, page_table: &mut PageTable, allocator: &mut Allocator) -> Result<u64, &'static str> {
    let stack_addr = allocator.allocate(stack_depth)
        .expect("Failed to allocate memory for stack");

    log::info!("Mapping stack");
    for i in 0..stack_depth {
        let ptr = stack_addr + i as u64 * 4096;
        unsafe {
            map_address(page_table, ptr, ptr, allocator)
                .expect("Failed to map stack");
        }
    }
    Ok(stack_addr + (stack_depth as u64 - 1) * 4096)
}

#[entry]
fn efi_main(image: uefi::Handle, mut system_table: uefi::table::SystemTable<uefi::table::Boot>) -> uefi::Status {
    let framebuffer = init_framebuffer(image, &mut system_table)
        .expect("Failed to init framebuffer");

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(framebuffer));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    log::info!("This is a very simple UEFI bootloader");

    let kernel_max_size = 10 * 4096;
    let kernel = load_kernel(image, &mut system_table, kernel_max_size)
        .expect("Failed to load kernel");

    let elf_file = ElfFile::new(unsafe { from_raw_parts(kernel, kernel_max_size) }).unwrap();
    header::sanity_check(&elf_file).expect("Failed to parse kernel file. Expected ELF");

    log::info!("Exiting boot services...");
    let (_runtime_system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);

    let mut allocator = init_allocator(memory_map)
        .expect("Failed to create Allocator");

    // convert to and from raw ptr to bypass borrow checker
    let page_table = unsafe {
        let page_table_ptr = allocator.allocate_page_table()
            .expect("Failed to allocate page table")
            as *mut PageTable;

        &mut *page_table_ptr
    };

    map_kernel(&elf_file, kernel as u64, page_table, &mut allocator)
        .expect("Failed to map kernel");

    map_framebuffer(&framebuffer, page_table, &mut allocator)
        .expect("Failed to map framebuffer");

    let stack = create_stack(20, page_table, &mut allocator)
        .expect("Failed to create stack");

    let entry_point = elf_file.header.pt2.entry_point();

    log::info!("Page table: {:#x}", page_table as *const PageTable as u64);
    log::info!("rsp: {:#x}", stack);
    log::info!("Jumping to kernel entry point at {:#x}", entry_point);
    log::info!("Kernel address: {:#x}", kernel as u64);
    log::info!("FB info: {:#x}", &framebuffer as *const _ as u64);

    unsafe {
        let ctx_switch_ptr = context_switch as *const () as u64;
        map_address(page_table, align_down(ctx_switch_ptr), align_down(ctx_switch_ptr), &mut allocator)
            .expect("Failed to map context switch function");

        context_switch(page_table as *const PageTable as u64, entry_point, stack, &framebuffer);
    }
}

unsafe fn context_switch(page_table: u64, entry_point: u64, stack_top: u64, frame_buffer_info: &FrameBufferInfo) -> ! {
    asm!(
    "mov cr3, {}; mov rsp, {}; push 0; jmp {}",
    in(reg) page_table,
    in(reg) stack_top,
    in(reg) entry_point,
    in("rdi") frame_buffer_info as *const _ as u64,
    );

    unreachable!();
}
