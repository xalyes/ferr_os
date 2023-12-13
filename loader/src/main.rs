#![feature(ptr_metadata)]
#![no_std]
#![no_main]

extern crate shared_lib;

use core::panic::PanicInfo;
use core::arch::asm;
use core::ptr::read_volatile;
use core::slice::from_raw_parts_mut;

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
use core::fmt::Write;
use core::ops::DivAssign;
use core::ptr::write;
use log::log;
use uefi::proto::console::text::Output;
use uefi::proto::media::file::File;
use uefi::table::boot::{OpenProtocolAttributes, OpenProtocolParams, AllocateType, MemoryType};
use uefi::table::SystemTable;
use uefi::proto::media::{
    file::{FileMode, FileAttribute, RegularFile},
    fs::SimpleFileSystem
};
use uefi::data_types::CStr16;
use uefi::proto::console::gop::GraphicsOutput;
use xmas_elf::{ElfFile, header, program};
use shared_lib::logger::LOGGER;
use shared_lib::logger::{FrameBufferInfo, PixelFormat};
use shared_lib::{logger, PageTable, PageTableEntry, PageTablesAllocator, map_address, get_physical_address};
use shared_lib::allocator::MemoryRegion;

fn convert_memory_type(t: MemoryType) -> shared_lib::allocator::MemoryType {
    match t {
        MemoryType::MMIO_PORT_SPACE | MemoryType::MMIO
        | MemoryType::RESERVED | MemoryType::UNUSABLE => shared_lib::allocator::MemoryType::RESERVED,

        MemoryType::PERSISTENT_MEMORY | MemoryType::CONVENTIONAL
        | MemoryType::LOADER_DATA | MemoryType::LOADER_CODE
        | MemoryType::BOOT_SERVICES_CODE | MemoryType::BOOT_SERVICES_DATA => shared_lib::allocator::MemoryType::FREE,

        MemoryType::ACPI_NON_VOLATILE | MemoryType::RUNTIME_SERVICES_CODE
        | MemoryType::RUNTIME_SERVICES_DATA => shared_lib::allocator::MemoryType::ACPI_1_3,

        MemoryType::ACPI_RECLAIM => shared_lib::allocator::MemoryType::ACPI_RECLAIM,

        MemoryType::PAL_CODE => shared_lib::allocator::MemoryType::ACPI_1_4,

        x => panic!("Unexpected memory type")
    }
}

#[entry]
fn efi_main(image: uefi::Handle, mut system_table: uefi::table::SystemTable<uefi::table::Boot>) -> uefi::Status {
    let mut framebuffer = {
        let gop_handle = system_table
            .boot_services()
            .get_handle_for_protocol::<GraphicsOutput>()
            .unwrap();
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
                .unwrap()
        };

        let mode_info = gop.current_mode_info();

        shared_lib::logger::FrameBufferInfo{
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
        }
    };

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(framebuffer.clone()));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    log::info!("This is UEFI bootloader");

    let kernel = {
        let fs_handle = system_table
            .boot_services()
            .get_handle_for_protocol::<SimpleFileSystem>()
            .unwrap();

        let mut fs = system_table
            .boot_services()
            .open_protocol_exclusive::<SimpleFileSystem>(fs_handle)
            .unwrap();

        let mut root_fs = fs.open_volume().unwrap();

        let mut buffer = [0; 1024];

        log::info!("Root filesystem:");
        log::info!("/");
        loop {
            match root_fs.read_entry(&mut buffer) {
                Ok(res) => {
                    match res {
                        None => break,
                        Some(file_info) => {
                            log::info!("\t {} - {}",
                            file_info.file_name(),
                            file_info.file_size())
                        }
                    }
                }
                Err(e) => {
                    panic!("Too small buffer");
                }
            }
        }

        let mut buff16: [u16; 16] = [0; 16];
        let mut kernel_name = CStr16::from_str_with_buf("kernel", &mut buff16)
            .expect("Failed to create CStr16");
        let handle = root_fs.open(kernel_name, FileMode::Read, FileAttribute::READ_ONLY)
            .expect("Failed to open kernel file");

        let mut file = unsafe { RegularFile::new(handle) };

        let kernel = {
            let ptr = system_table
                .boot_services()
                .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 10)
                .expect("Failed to allocate page for kernel");
            unsafe { from_raw_parts_mut(ptr as *mut u8, 40960) }
        };

        file.read(kernel).expect("Failed to read kernel file");
        kernel
    };

    log::info!("Exiting boot services...");
    let (runtime_system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);

    let memory_map_size = core::mem::size_of::<shared_lib::allocator::MemoryRegion>() * memory_map.entries().count();
    let pages_to_find = 1 + (memory_map_size / 4096);
    log::info!("Finding region for loader's memory map... Needed size: {} - {} pages", memory_map_size, pages_to_find);

    let mut find_result: Option<(u64, usize)> = None;
    for memory_descriptor in memory_map.entries() {
        if memory_descriptor.page_count >= pages_to_find as u64
            && matches!(convert_memory_type(memory_descriptor.ty), shared_lib::allocator::MemoryType::FREE)
            && memory_descriptor.phys_start != 0 {
            find_result = Option::from((memory_descriptor.phys_start, memory_descriptor.page_count as usize));
            break;
        }
    }

    log::info!("Found suitable memory region: addr: {:#x} pages: {}", find_result.unwrap().0, find_result.unwrap().1);

    let mut loader_memory_map: &mut [MemoryRegion] = unsafe { from_raw_parts_mut(find_result.unwrap().0 as *mut MemoryRegion, pages_to_find * 4096) };

    for (idx, memory_descriptor) in memory_map.entries().enumerate() {
        if memory_descriptor.phys_start == find_result.unwrap().0 {
            if pages_to_find < memory_descriptor.page_count as usize {
                loader_memory_map[idx] = shared_lib::allocator::MemoryRegion {
                    ty: shared_lib::allocator::MemoryType::FREE,
                    addr: memory_descriptor.phys_start + (4096 * pages_to_find) as u64,
                    page_count: memory_descriptor.page_count as usize - pages_to_find
                };
                continue;
            }
        }

        if memory_descriptor.phys_start == 0 {
            loader_memory_map[idx] = shared_lib::allocator::MemoryRegion {
                ty: shared_lib::allocator::MemoryType::RESERVED,
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
        ty: shared_lib::allocator::MemoryType::IN_USE,
        addr: find_result.unwrap().0,
        page_count: pages_to_find
    };

    for i in 0..10 {
        log::info!("Loader memory map entry addr: {:#x}, page_count: {}, type: {:?}",
            loader_memory_map[i].addr, loader_memory_map[i].page_count, loader_memory_map[i].ty);
    }

    log::info!("Loader memory map entry addr: {:#x}, page_count: {}, type: {:?}",
            loader_memory_map[memory_map.entries().len()].addr, loader_memory_map[memory_map.entries().len()].page_count, loader_memory_map[memory_map.entries().len()].ty);

    let mut allocator = shared_lib::allocator::Allocator::new(loader_memory_map, memory_map.entries().len() + 1);

    let l4_page_table = allocator.allocate_page_table().unwrap() as *mut PageTable;
    unsafe {
        map_address(&mut *l4_page_table, context_switch as *const () as u64, context_switch as *const () as u64, &mut allocator)
            .expect("Failed to map context switch function");

        let phys = get_physical_address(& *l4_page_table, context_switch as *const () as u64)
            .expect("Failed to get context switch phys addr");

        let fb_start = framebuffer.addr;
        let fb_end = framebuffer.addr + framebuffer.size as u64 - 1;
        let pages_needed_for_fb = framebuffer.size / 4096;
        log::info!("Mapping framebuffer. addr: {:#x} - {:#x}. Pages for framebuffer: {}", fb_start, fb_end, pages_needed_for_fb);

        for i in 0..pages_needed_for_fb {
            let ptr = fb_start + i as u64 * 4096;
            map_address(&mut *l4_page_table, ptr, ptr, &mut allocator)
                .expect("Failed to map framebuffer");
        }

        let fb_info_ptr = &framebuffer as *const _ as u64;
        log::info!("Mapping framebuffer info. addr: {:#x}", fb_info_ptr);
        map_address(&mut *l4_page_table, fb_info_ptr, fb_info_ptr, &mut allocator)
            .expect("Failed to map fb info");

        let stack_addr = allocator.allocate(21)
            .expect("Failed to allocate memory for stack");

        log::info!("Mapping stack");
        for i in 0..21 {
            let ptr = stack_addr + i as u64 * 4096;
            map_address(&mut *l4_page_table, ptr, ptr, &mut allocator)
                .expect("Failed to map stack");
        }

        let elf_file = ElfFile::new(kernel).unwrap();
        header::sanity_check(&elf_file).expect("Failed to parse kernel file. Expected ELF");

        for header in elf_file.program_iter() {
            match header.get_type().unwrap() {
                program::Type::Load => {
                    let phys_start_addr = (kernel.as_ptr() as u64) + header.offset();
                    let phys_end_addr = phys_start_addr + header.file_size() - 1;

                    let virt_start_addr = header.virtual_addr();

                    log::info!("Handling segment phys: {:#x}, virt: {:#x}, phys_start: {:#x}, phys_end: {:#x}",
                    header.physical_addr(), virt_start_addr, phys_start_addr, phys_end_addr);

                    for i in 0..(1 + header.file_size() / 4096) {
                        log::info!("Mapping {:#x} to {:#x}", virt_start_addr + i * 4096, phys_start_addr + i * 4096);
                        map_address(&mut *l4_page_table, virt_start_addr + i * 4096, phys_start_addr + i * 4096, &mut allocator)
                            .expect("Failed to map kernel");
                    }
                }
                program::Type::Tls => { panic!("Not implemented TLS section") }
                _ => {}
            }
        }

        let entry_point = elf_file.header.pt2.entry_point();

        log::info!("Page table: {:#x}", l4_page_table as u64);
        log::info!("Context switch function page addr: {:#x}", phys);
        log::info!("rsp: {:#x}", stack_addr + 19*4096);
        log::info!("Jumping to kernel entry point at {:#x}", entry_point);
        log::info!("Kernel address: {:#x}", kernel.as_ptr() as u64);
        log::info!("FB info: {:#x}", &framebuffer as *const _ as u64);

        //map_address(&mut *l4_page_table, l4_page_table as u64, l4_page_table as u64, &mut allocator)
        //    .expect("Failed to map l4 page table");
        
        context_switch(l4_page_table as u64, entry_point, stack_addr + 19*4096, &framebuffer);
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
