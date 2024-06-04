use core::arch::asm;
use core::marker::PhantomData;

pub struct Port {
    port: u16,
    phantom: PhantomData<u8>,
}

impl Port {
    #[inline]
    pub const fn new(port: u16) -> Port {
        Port {
            port,
            phantom: PhantomData,
        }
    }

    #[inline]
    pub unsafe fn write(&mut self, value: u8) {
        unsafe {
            asm!("out dx, al", in("dx") self.port, in("al") value, options(nomem, nostack, preserves_flags));
        }
    }

    #[inline]
    pub unsafe fn write_u16(&mut self, value: u16) {
        unsafe {
            asm!("out dx, ax", in("dx") self.port, in("ax") value, options(nomem, nostack, preserves_flags));
        }
    }

    #[inline]
    pub unsafe fn write_u32(&mut self, value: u32) {
        unsafe {
            asm!("out dx, eax", in("dx") self.port, in("eax") value, options(nomem, nostack, preserves_flags));
        }
    }

    #[inline]
    pub unsafe fn read(&mut self) -> u8 {
        let value: u8;
        unsafe {
            asm!("in al, dx", out("al") value, in("dx") self.port, options(nomem, nostack, preserves_flags));
        }
        value
    }

    #[inline]
    pub unsafe fn read_u32(&mut self) -> u32 {
        let value: u32;
        unsafe {
            asm!("in eax, dx", out("eax") value, in("dx") self.port, options(nomem, nostack, preserves_flags));
        }
        value
    }
}

#[inline]
pub unsafe fn write(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

#[inline]
pub unsafe fn read(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}
