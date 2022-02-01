use bit_field::BitField;
use core::ops::Add;
use core::ops::Sub;

#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub struct PhysAddr(pub u64);

pub struct PhysFrame {
    addr: PhysAddr
}

impl PhysFrame {
    pub fn containing_address(mut addr: PhysAddr) -> Self {
        PhysFrame{ addr: PhysAddr(*addr.0.set_bits(0..12, 0)) }
    }
}

impl Add for PhysAddr {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        Self {
            0: self.0 + other.0
        }
    }
}

impl Add<u64> for PhysAddr {
    type Output = Self;

    fn add(self, other: u64) -> Self::Output {
        Self {
            0: self.0 + other
        }
    }
}

impl Sub<u64> for PhysAddr {
    type Output = Self;

    fn sub(self, other: u64) -> Self::Output {
        Self {
            0: self.0 - other
        }
    }
}