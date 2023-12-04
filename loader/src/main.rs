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
use core::ptr::write;
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
use shared_lib::{logger, map_address, PageTable, PageTableEntry, PageTablesAllocator};

struct UefiAllocator(SystemTable<uefi::table::Boot>);

impl shared_lib::PageTablesAllocator for UefiAllocator {
    fn allocate(&mut self) -> Result<&mut PageTable, &'static str> {
        let allocated_page = self.0.boot_services()
            .allocate_pages(uefi::table::boot::AllocateType::AnyPages, uefi::table::boot::MemoryType::LOADER_DATA, 1)
            .unwrap();

        let page_table_ptr = allocated_page as *mut PageTable;

        unsafe {
            core::ptr::write(page_table_ptr, PageTable::new());
            Ok(&mut (*page_table_ptr))
        }
    }

    unsafe fn get_mut_ptr(&mut self) -> *mut dyn PageTablesAllocator {
        return self as *mut dyn PageTablesAllocator;
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

    let logger = logger::LOGGER.get_or_init(move || logger::LockedLogger::new(framebuffer));
    log::set_logger(logger).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

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

    let mut pages_total = 0;
    for memory_descriptor in memory_map.entries() {
        log::info!("Memory region. page_count: {}, type: {}",
            memory_descriptor.page_count,
            memory_descriptor.ty.0,
        );
        pages_total += memory_descriptor.page_count;
    }

    log::info!("Pages total: {}", pages_total);

    loop {}

    let mut allocator = UefiAllocator{ 0: system_table };
    let mut l4_page_table = shared_lib::PageTable::new();
    let mut pages = 0;
    for memory_descriptor in memory_map.entries() {
        for i in 0..memory_descriptor.page_count {
            log::info!("Mapping... pages already mapped: {}", pages);
            unsafe {
                map_address(
                    &mut l4_page_table,
                    memory_descriptor.phys_start + i * 4096,
                    memory_descriptor.phys_start + i * 4096,
                    &mut allocator)
                    .unwrap()
            }
            pages += 1;
        }
    }

    log::info!("Done mapping. Pages total mapped {}", pages);

    /*
    system_table.stdout().clear().unwrap();
    writeln!(system_table.stdout(), "This is UEFI bootloader").unwrap();

    //let mut allocator = shared_lib::StaticAllocator::new();
    //let mut l4_page_table = shared_lib::PageTable::new();

    let entry_point = {
        let stdout = system_table.stdout();
        stdout.clear().unwrap();
        writeln!(stdout, "Kernel size: {}", KERNEL.len()).unwrap();

        let elf_file = ElfFile::new(KERNEL).unwrap();
        header::sanity_check(&elf_file).unwrap();

        for segment in elf_file.program_iter() {
            program::sanity_check(segment, &elf_file).unwrap();

            if let program::Type::Load = segment.get_type().unwrap() {
                writeln!(stdout, "Loading segment...").unwrap();
            }
        }

        let cr0: u32;
        unsafe {
            asm!(
            "mov {0:r}, cr0",
            out(reg) cr0
            );
        }
        writeln!(stdout, "CR0 value: {}", cr0).unwrap();
        elf_file.header.pt2.entry_point()
    };

    writeln!(system_table.stdout(), "entry point: {}", entry_point).unwrap();

*/

    loop{}
    //unsafe { context_switch(entry_point, 0x20000, framebuffer); }
}

unsafe fn context_switch(entry_point: u64, stack_top: u64, frame_buffer_info: FrameBufferInfo) -> ! {
    asm!(
    "mov rsp, {}; push 0; jmp {}",
    in(reg) stack_top,
    in(reg) entry_point,
    in("rdi") &frame_buffer_info as *const _ as usize,
    );

    unreachable!();
}
