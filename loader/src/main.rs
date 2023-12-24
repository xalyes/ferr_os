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
use shared_lib::addr::VirtAddr;
use shared_lib::logger::FrameBufferInfo;
use shared_lib::page_table::{PageTable, PageTablesAllocator, map_address, remap_address, align_down, align_down_u64, PageTableFlags, get_physical_address};
use shared_lib::{logger, VIRT_MAPPING_OFFSET};
use shared_lib::allocator::{MemoryRegion, Allocator, MemoryMap};

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

unsafe fn init_allocator(memory_map: uefi::table::boot::MemoryMap)
                         -> Result<shared_lib::allocator::Allocator, &'static str> {

    static mut MMAP_ARRAY: [MemoryRegion; shared_lib::allocator::MAX_MEMORY_MAP_SIZE]
        = [MemoryRegion{ ty: shared_lib::allocator::MemoryType::Reserved, addr: 0, page_count: 0 }; shared_lib::allocator::MAX_MEMORY_MAP_SIZE];

    log::info!("len {}", memory_map.entries().len());
    for (idx, memory_descriptor) in memory_map.entries().enumerate() {
        if memory_descriptor.phys_start == 0 {
            MMAP_ARRAY[idx] = shared_lib::allocator::MemoryRegion {
                ty: shared_lib::allocator::MemoryType::Reserved,
                addr: memory_descriptor.phys_start,
                page_count: memory_descriptor.page_count as usize
            };
            continue;
        }

        MMAP_ARRAY[idx] = shared_lib::allocator::MemoryRegion {
            ty: convert_memory_type(memory_descriptor.ty),
            addr: memory_descriptor.phys_start,
            page_count: memory_descriptor.page_count as usize
        };
    }

    for i in 0..64 {
        log::info!("Loader memory map entry addr: {:#x}, page_count: {}, type: {:?}",
            MMAP_ARRAY[i].addr, MMAP_ARRAY[i].page_count, MMAP_ARRAY[i].ty);
    }

    let memory_map: MemoryMap = MemoryMap{ entries: &mut MMAP_ARRAY, next_free_entry_idx: (memory_map.entries().len()) as u64 };
    Ok(shared_lib::allocator::Allocator::new(memory_map))
}

#[derive(Copy, Clone)]
struct MappedEntry {
    pub page: VirtAddr,
    pub frame: u64
}

fn map_kernel(elf_file: &ElfFile, kernel: u64, page_table: &mut PageTable, allocator: &mut Allocator) -> Result<(), &'static str> {
    let mut mapped_frames: [MappedEntry; 100] = [ MappedEntry{ page: VirtAddr::zero(), frame: 0 }; 100 ];
    let mut mapped_frames_counter = 0;

    for header in elf_file.program_iter() {
        match header.get_type().unwrap() {
            program::Type::Load => {
                let phys_start_addr = (kernel as u64) + header.offset();
                let phys_end_addr = phys_start_addr + header.file_size() - 1;

                let virt_start_addr = VirtAddr::new_checked(header.virtual_addr())
                    .expect("Got bad virtual address from ELF");

                log::info!("[kernel map] segment: {}, phys_start: {:#x}, phys_end: {:#x}",
                    virt_start_addr, phys_start_addr, phys_end_addr);

                let virt_start_addr_aligned = align_down(virt_start_addr);
                let phys_start_addr_aligned = align_down_u64(phys_start_addr);

                for i in 0..(1 + (header.file_size() - 1 + virt_start_addr.0 - virt_start_addr_aligned.0) / 4096) {
                    let virt = virt_start_addr_aligned.offset(i * 4096).unwrap();
                    let phys = phys_start_addr_aligned + i * 4096;

                    log::info!("[kernel map] Mapping {} to {:#x}", virt, phys);
                    unsafe {
                        map_address(page_table, virt, phys, allocator)
                            .expect("Failed to map kernel");
                    }
                    mapped_frames[mapped_frames_counter] = MappedEntry{ page: virt, frame: phys };
                    mapped_frames_counter += 1;
                }

                if header.mem_size() > header.file_size() {
                    let zero_start = virt_start_addr.offset(header.file_size()).unwrap();
                    let zero_end = virt_start_addr.offset(header.mem_size()).unwrap();

                    log::info!("[kernel map] .bss section: from {} to {}. size: {}", zero_start, zero_end, header.mem_size() - header.file_size());

                    let data_bytes_before_zero = zero_start.0 & 0xfff;

                    if data_bytes_before_zero != 0 {
                        let frame = allocator.allocate(1).expect("Failed to allocate new frame");
                        unsafe {
                            let frame_to_copy = align_down_u64(phys_end_addr);
                            for i in 0..mapped_frames_counter {
                                if mapped_frames[i].frame == frame_to_copy {
                                    log::info!("[kernel map] Remapping {} to {:#x}", mapped_frames[i].page, frame);
                                    remap_address(page_table, mapped_frames[i].page, frame, allocator)
                                        .expect("Failed to map kernel");
                                }
                            }

                            log::info!("[kernel map] Copying from {:#x}", align_down_u64(phys_end_addr));
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
                    }

                    if header.mem_size() - header.file_size() > (4096 - data_bytes_before_zero) {
                        let zero_start_aligned = zero_start.offset(4096 - data_bytes_before_zero).unwrap();
                        let bytes_to_allocate = header.mem_size() - header.file_size() - (4096 - data_bytes_before_zero);
                        log::info!("[kernel map] bytes_to_allocate: {}", bytes_to_allocate);

                        for i in 0..(1 + bytes_to_allocate / 4096) {
                            let frame = allocator.allocate(1).expect("Failed to allocate new frame");
                            let virt_ptr = zero_start_aligned.offset(i * 4096).unwrap();
                            log::info!("[kernel map] Mapping {} to {:#x}", virt_ptr, frame);

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

fn map_framebuffer(framebuffer: &FrameBufferInfo, page_table: &mut PageTable, allocator: &mut Allocator) -> Result<(), &'static str> {
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

    let fb_info_ptr = framebuffer as *const _ as u64;
    log::info!("Mapping framebuffer info. addr: {:#x}", fb_info_ptr);
    unsafe {
        map_address(page_table, align_down(VirtAddr::new_checked(fb_info_ptr).unwrap()), align_down_u64(fb_info_ptr), allocator)
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
            map_address(page_table, VirtAddr::new_checked(ptr).unwrap(), ptr, allocator)
                .expect("Failed to map stack");
        }
    }
    Ok(stack_addr + (stack_depth as u64 - 1) * 4096)
}

#[entry]
fn efi_main(image: uefi::Handle, mut system_table: uefi::table::SystemTable<uefi::table::Boot>) -> uefi::Status {
    let mut framebuffer = init_framebuffer(image, &mut system_table)
        .expect("Failed to init framebuffer");

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(framebuffer));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    log::info!("This is a very simple UEFI bootloader");

    let kernel_max_size = 20 * 4096;
    let kernel = load_kernel(image, &mut system_table, kernel_max_size)
        .expect("Failed to load kernel");

    let elf_file = ElfFile::new(unsafe { from_raw_parts(kernel, kernel_max_size) }).unwrap();
    header::sanity_check(&elf_file).expect("Failed to parse kernel file. Expected ELF");

    log::info!("Exiting boot services...");
    let (_runtime_system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);

    let last_memory_region = memory_map.entries().last().unwrap();
    let last_frame_addr = last_memory_region.phys_start + (last_memory_region.page_count - 1) * 4096;

    let mut allocator = unsafe { init_allocator(memory_map)
        .expect("Failed to create Allocator") };

    // convert to and from raw ptr to bypass borrow checker
    let page_table = unsafe {
        let page_table_ptr = allocator.allocate_page_table()
            .expect("Failed to allocate page table")
            as *mut PageTable;

        &mut *page_table_ptr
    };

    log::info!("Mapping all memory. Last frame: {:#x}", last_frame_addr);

    for i in 0..(last_frame_addr / 4096) {
        let phys =  i * 4096;
        let virt = VirtAddr::new(phys + VIRT_MAPPING_OFFSET);

        unsafe {
            map_address(page_table, virt, phys, &mut allocator)
                .expect("Failed to map memory");
        }
    }

    map_kernel(&elf_file, kernel as u64, page_table, &mut allocator)
        .expect("Failed to map kernel");

    map_framebuffer(&framebuffer, page_table, &mut allocator)
        .expect("Failed to map framebuffer");
    framebuffer.addr += VIRT_MAPPING_OFFSET;

    let stack = create_stack(20, page_table, &mut allocator)
        .expect("Failed to create stack");

    let entry_point = elf_file.header.pt2.entry_point();

    log::info!("Page table: {:#x}", page_table as *const PageTable as u64);
    log::info!("rsp: {:#x}", stack);
    log::info!("Jumping to kernel entry point at {:#x}", entry_point);
    log::info!("Kernel address: {:#x}", kernel as u64);
    log::info!("FB addr: {:#x}", framebuffer.addr);
    log::info!("FB info: {:#x}", &framebuffer as *const _ as u64);

    unsafe {
        let ctx_switch_ptr = context_switch as *const () as u64;
        map_address(page_table, align_down(VirtAddr::new_checked(ctx_switch_ptr).unwrap()), align_down_u64(ctx_switch_ptr), &mut allocator)
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
