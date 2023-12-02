#![feature(ptr_metadata)]
#![no_std]
#![no_main]

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

static KERNEL: &[u8] = include_bytes!("/home/max/rust_os_3/build/kernel");

enum PixelFormat {
    Rgb,
    Bgr,
    Bitmask,
    BltOnly
}

struct FrameBufferInfo {
    addr: u64,
    size: usize,
    width: usize,
    height: usize,
    pixel_format: PixelFormat,
    stride: usize
}

fn write_pixel(fb_info: &FrameBufferInfo, fb_slice: &mut [u8], x: usize, y: usize, intensity: u8) {
    let pixel_offset = y * fb_info.stride + x;
    let color = match &fb_info.pixel_format {
        PixelFormat::Rgb => [intensity, intensity, intensity / 2, 0],
        PixelFormat::Bgr => [intensity / 2, intensity, intensity, 0],
        other => {
            loop {}
        }
    };
    let bytes_per_pixel = 4;
    let byte_offset = pixel_offset * bytes_per_pixel;
    fb_slice[byte_offset..(byte_offset + bytes_per_pixel)]
        .copy_from_slice(&color[..bytes_per_pixel]);
    let _ = unsafe { read_volatile(&fb_slice[byte_offset]) };
}

fn render_char_15(c: char) -> [u8; 15] {
    let f = 255;

    match c {
        '0' => [0, f, 0,
            f, 0, f,
            f, 0, f,
            f, 0, f,
            0, f, 0],
        '1' => [0, f, 0,
            f, f, 0,
            0, f, 0,
            0, f, 0,
            f, f, f],
        '2' => [0, f, 0,
            f, 0, f,
            0, 0, f,
            0, f, 0,
            f, f, f],
        '3' => [f, f, f,
            0, 0, f,
            f, f, f,
            0, 0, f,
            f, f, f],
        '4' => [f, 0, f,
            f, 0, f,
            f, f, f,
            0, 0, f,
            0, 0, f],
        '5' => [f, f, f,
            f, 0, 0,
            f, f, f,
            0, 0, f,
            f, f, f],
        '6' => [f, f, f,
            f, 0, 0,
            f, f, f,
            f, 0, f,
            f, f, f],
        '7' => [f, f, f,
            0, 0, f,
            0, 0, f,
            0, f, 0,
            f, 0, 0],
        '8' => [f, f, f,
            f, 0, f,
            f, f, f,
            f, 0, f,
            f, f, f],
        '9' => [f, f, f,
            f, 0, f,
            f, f, f,
            0, 0, f,
            f, f, f],
        other =>
            [f, f, f,
                f, f, f,
                f, f, f,
                f, f, f,
                f, f, f],
    }
}

fn render_char_96(c: char) -> [u8; 96] {
    let f: u8 = 255;

    match c {
        '7' => [f,f,f,f,f,f,f,f,
            f,0,0,0,0,0,f,f,
            0,0,0,0,0,0,f,0,
            0,0,0,0,0,f,0,0,
            0,0,0,0,f,0,0,0,
            0,0,0,f,0,0,0,0,
            0,0,0,f,0,0,0,0,
            0,0,f,0,0,0,0,0,
            0,0,f,0,0,0,0,0,
            0,f,0,0,0,0,0,0,
            0,f,0,0,0,0,0,0,
            f,0,0,0,0,0,0,0,],
        other=> [f; 96]
    }
}

#[entry]
fn efi_main(image: uefi::Handle, mut system_table: uefi::table::SystemTable<uefi::table::Boot>) -> uefi::Status {
    let entry_point = {
        let stdout = system_table.stdout();
        stdout.clear().unwrap();
        writeln!(stdout, "This is UEFI bootloader").unwrap();
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

        FrameBufferInfo{
            addr: gop.frame_buffer().as_mut_ptr() as u64,
            size: gop.frame_buffer().size(),
            width: mode_info.resolution().0,
            height: mode_info.resolution().1,
            pixel_format: match mode_info.pixel_format() {
                uefi::proto::console::gop::PixelFormat::Rgb => PixelFormat::Rgb,
                uefi::proto::console::gop::PixelFormat::Bgr => PixelFormat::Bgr,
                uefi::proto::console::gop::PixelFormat::Bitmask => PixelFormat::Bitmask,
                uefi::proto::console::gop::PixelFormat::BltOnly => PixelFormat::BltOnly
            },
            stride: mode_info.stride()
        }
    };

    writeln!(system_table.stdout(), "exiting boot services...").unwrap();

    let _res = system_table.exit_boot_services(uefi::table::boot::MemoryType::BOOT_SERVICES_DATA);

    let fb_slice = unsafe { from_raw_parts_mut(framebuffer.addr as *mut u8, framebuffer.size) };
    fb_slice.fill(0);

    let x_pos = 1;
    let y_pos = 1;
    let seven_rendered = render_char_96('7');
    for (idx, intensity) in seven_rendered.iter().enumerate() {
        write_pixel(&framebuffer, fb_slice, x_pos + (idx % 8), y_pos + (idx / 8), *intensity);
    }

    /*for i in 0..framebuffer.height {
        write_pixel(&framebuffer, fb_slice, i, i, 255);
    }*/
    /*for i in 1..200 {
        write_pixel(&framebuffer, fb_slice, i, i, 255);
    }*/

    unsafe { context_switch(entry_point, 0x20000, framebuffer); }
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
