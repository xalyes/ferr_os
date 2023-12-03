#![feature(ptr_metadata)]
#![no_std]
#![no_main]

extern crate shared_lib;

use core::panic::PanicInfo;
use core::arch::asm;
use core::ptr::read_volatile;
use core::slice::from_raw_parts_mut;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

use uefi::prelude::entry;
use core::fmt::Write;
use core::ptr::write;
use uefi::proto::console::text::Output;
use uefi::table::boot::{OpenProtocolAttributes, OpenProtocolParams};
use xmas_elf::{ElfFile, header, program};
use shared_lib::logger::LOGGER;
use shared_lib::logger::{FrameBufferInfo, PixelFormat};
use shared_lib::logger;

static KERNEL: &[u8] = include_bytes!("/home/max/rust_os_3/build/kernel");

#[entry]
fn efi_main(image: uefi::Handle, mut system_table: uefi::table::SystemTable<uefi::table::Boot>) -> uefi::Status {
    let mut framebuffer = {
        let gop_handle = system_table
            .boot_services()
            .get_handle_for_protocol::<uefi::proto::console::gop::GraphicsOutput>()
            .unwrap();
        let mut gop = unsafe {
            system_table.boot_services()
                .open_protocol::<uefi::proto::console::gop::GraphicsOutput>(
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

    let (system_table, memory_map) = system_table
        .exit_boot_services(uefi::table::boot::MemoryType::LOADER_DATA);

    let mut pages = 0;
    for memory_descriptor in memory_map.entries() {
        log::info!("Memory region. type: {}, attr: 0x{:x?}, page_count: {}, phys: 0x{:x?}, virt: 0x{:x?}",
                 memory_descriptor.ty.0,
                 memory_descriptor.att.bits(),
                 memory_descriptor.page_count,
                 memory_descriptor.phys_start,
                 memory_descriptor.virt_start
        );
        pages += memory_descriptor.page_count;
    }

    log::info!("Pages total {}", pages);

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
