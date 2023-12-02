#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

mod serial;

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
    where
        T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

#[cfg(test)]
fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }

    exit_qemu(QemuExitCode::Success);
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    unsafe {
        let port = 0xf4;
        let value = exit_code as u8;
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

use core::panic::PanicInfo;
use core::arch::asm;
use core::fmt::Write;
use core::ptr::read_volatile;
use core::slice::from_raw_parts_mut;
use crate::serial::SERIAL1;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

// our panic handler in test mode
#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    loop {}
}

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

#[no_mangle]
#[cfg(test)]
pub extern "C" fn _start() -> ! {
    test_main();

    loop {}
}

#[macro_export]
macro_rules! entry_point {
    ($path:path) => {
        #[export_name = "_start"]
        pub extern "C" fn __impl_start(fb_info: &'static mut $crate::FrameBufferInfo) -> ! {
            // validate the signature of the program entry point
            let f: fn(&'static mut $crate::FrameBufferInfo) -> ! = $path;

            f(fb_info)
        }
    };
}

#[cfg(not(test))]
entry_point!(kernel_main);

#[cfg(not(test))]
fn kernel_main(frame_buffer_info: &'static mut FrameBufferInfo) -> ! {
    let fb_slice = unsafe { from_raw_parts_mut(frame_buffer_info.addr as *mut u8, frame_buffer_info.size) };
    fb_slice.fill(0);

    let x_pos = 1;
    let y_pos = 1;
    let seven_rendered = render_char_96('7');
    for (idx, intensity) in seven_rendered.iter().enumerate() {
        write_pixel(&frame_buffer_info, fb_slice, x_pos + (idx % 8), y_pos + (idx / 8), *intensity);
    }

    for i in 1..1000 {
        write_pixel(&frame_buffer_info, fb_slice, i, i, 255);
    }

    loop {
        unsafe { asm!("hlt") };
    }
}
