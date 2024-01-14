use core::arch::asm;
use core::ptr;
use crate::idt::{InterruptStackFrame, InterruptDescriptorTable, PageFaultErrorCode};
use lazy_static::lazy_static;
use crate::gdt;
use spin;
use crate::pic::{ChainedPics, Port};
use crate::apic::Apic;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    Spurious = 39
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

pub static PICS: spin::Mutex<ChainedPics> =
    spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub static APIC: spin::Mutex<Apic> =
    spin::Mutex::new(Apic::new());

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault.set_handler_fn(double_fault_handler).set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
        idt[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
        idt[InterruptIndex::Spurious.as_usize()].set_handler_fn(spurious_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);

        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(
    stack_frame: InterruptStackFrame)
{
    log::info!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame, _error_code: u64) -> !
{
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame, error_code: u64)
{
    log::info!("EXCEPTION: GENERAL PROTECTION FAULT\n{:#?}. Error code: {}", stack_frame, error_code);
}

extern "x86-interrupt" fn timer_interrupt_handler(
    _stack_frame: InterruptStackFrame)
{
    unsafe {
        static mut I: u64 = 0;
        if I % 100 == 0 {
            log::info!("1 sec timer tick. {}", I / 100);
        }
        I += 1;
    }

    unsafe {
        APIC.lock()
            .notify_end_of_interrupt();
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(
    _stack_frame: InterruptStackFrame)
{
    let mut port = Port::new(0x60);
    let scancode = unsafe { port.read() };
    crate::task::keyboard::add_scancode(scancode);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    unsafe {
        shared_lib::logger::LOGGER
            .get()
            .map(|l| l.force_unlock())
    };

    log::info!("EXCEPTION: PAGE FAULT");

    let cr2: u64;
    unsafe {
        asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack, preserves_flags));
    }

    log::info!("Accessed Address: {:#x}", cr2);
    log::info!("Error Code: {:?}", error_code);
    log::info!("{:#?}", stack_frame);

    log::info!("Reading stack from address {:#x}", stack_frame.value.stack_pointer.0);
    unsafe {
        for i in 0..40 {
            log::info!("{}: {:#x}", i, ptr::read_volatile((stack_frame.value.stack_pointer.0 + 8 * i) as *const u64));
        }
    }

    loop {
        unsafe {
            asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

extern "x86-interrupt" fn spurious_handler(
    _stack_frame: InterruptStackFrame)
{
}
