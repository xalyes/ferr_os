#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(rust_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use rust_os::{ FrameBufferInfo, PixelFormat, entry_point };

mod serial;

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
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
    rust_os::test_panic_handler(info);
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

#[cfg(test)]
entry_point!(test_kernel_main);

#[cfg(test)]
fn test_kernel_main(_fb_info: &'static mut FrameBufferInfo) -> ! {
    test_main();
    loop {}
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
