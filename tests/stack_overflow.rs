#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use lazy_static::lazy_static;
use shared_lib::serial_print;
use shared_lib::{exit_qemu, QemuExitCode};
use rust_os::idt::{InterruptStackFrame, InterruptDescriptorTable };

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_print!("stack_overflow::stack_overflow...\t");

    rust_os::gdt::init();
    init_test_idt();

    // trigger a stack overflow
    stack_overflow();

    panic!("Execution continued after stack overflow");
}

#[inline(never)]
#[allow(unconditional_recursion)]
fn stack_overflow() {
    stack_overflow(); // for each recursion, the return address is pushed
    stack_overflow();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    rust_os::test_panic_handler(info)
}

lazy_static! {
    static ref TEST_IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        unsafe {
            idt.double_fault
                .set_handler_fn(test_double_fault_handler)
                .set_stack_index(rust_os::gdt::DOUBLE_FAULT_IST_INDEX);
        }

        idt
    };
}

extern "x86-interrupt" fn test_double_fault_handler(
    _stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_print!("[ok]\n");
    exit_qemu(QemuExitCode::Success);
    loop {}
}

pub fn init_test_idt() {
    TEST_IDT.load();
}