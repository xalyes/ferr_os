use core::arch::asm;
use crate::bits::get_bits;

#[inline]
pub fn without_interrupts<F, R>(f: F) -> R
    where
        F: FnOnce() -> R,
{
    let rflags: u64;

    unsafe {
        asm!("pushfq; pop {}", out(reg) rflags, options(nomem, preserves_flags));
    }

    // true if the interrupt flag is set (i.e. interrupts are enabled)
    let saved_intpt_flag = get_bits(rflags, 9..10) == 1;

    // if interrupts are enabled, disable them for now
    if saved_intpt_flag {
        unsafe { asm!("cli", options(nomem, nostack)); }
    }

    // do `f` while interrupts are disabled
    let ret = f();

    // re-enable interrupts if they were previously enabled
    if saved_intpt_flag {
        unsafe { asm!("sti", options(nomem, nostack)); }
    }

    // return the result of `f` to the caller
    ret
}