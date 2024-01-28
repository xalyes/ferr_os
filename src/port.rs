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
    pub unsafe fn read(&mut self) -> u8 {
        let value: u8;
        unsafe {
            asm!("in al, dx", out("al") value, in("dx") self.port, options(nomem, nostack, preserves_flags));
        }
        value
    }
}
