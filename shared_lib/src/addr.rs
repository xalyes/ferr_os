use core::fmt;
use core::fmt::Formatter;
use core::ops::{Add, BitAnd};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PhysAddr(pub u64);

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct VirtAddr(pub u64);

impl VirtAddr {
    /// Create a new canonical virtual address.
    #[inline]
    pub fn new(addr: u64) -> VirtAddr {
        // By doing the right shift as a signed operation (on a i64), it will
        // sign extend the value, repeating the leftmost bit.
        VirtAddr(((addr << 16) as i64 >> 16) as u64)
    }

    /// Tries to create a new canonical virtual address.
    ///
    /// This function tries to performs sign
    /// extension of bit 47 to make the address canonical. It succeeds if bits 48 to 64 are
    /// either a correct sign extension (i.e. copies of bit 47) or all null. Else, an error
    /// is returned.
    #[inline]
    pub fn new_checked(addr: u64) -> Result<VirtAddr, &'static str> {
        match addr & 0xffff_8000_0000_0000 {
            0 | 0xffff_8000_0000_0000 => Ok(VirtAddr(addr)),     // address is canonical
            0x0000_8000_0000_0000 => Ok(VirtAddr::new(addr)), // address needs sign extension
            _ => Err("Virt addr not valid"),
        }
    }

    #[inline]
    pub fn offset(&self, offset: u64) -> Result<VirtAddr, &'static str> {
        let (result, overflow) = self.0.overflowing_add(offset);
        if overflow {
            return Err("Virt addr overflow");
        }
        Ok(VirtAddr::new(result))
    }

    #[inline]
    pub fn from_ptr<T: ?Sized>(ptr: *const T) -> Self {
        Self::new(ptr as *const () as u64)
    }

    #[inline]
    pub const fn zero() -> VirtAddr {
        VirtAddr(0)
    }
}

impl BitAnd for VirtAddr {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self::new(self.0 & rhs.0)
    }
}

impl Add for VirtAddr {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.0 + rhs.0)
    }
}

impl fmt::Display for VirtAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}

#[test_case]
fn check_sign_extension() {
    let virt_positive = VirtAddr::new(0xf000_0000_0000_0023);
    assert_eq!(0x0000_0000_0000_0023, virt_positive.0);

    let virt_negative = VirtAddr::new(0xffff_800f_0000_0023);
    assert_eq!(0xffff_800f_0000_0023, virt_negative.0);
}

#[test_case]
fn check_ctr_new_checked() {
    let virt1 = VirtAddr::new_checked(0x0222).unwrap();
    assert_eq!(0x0222, virt1.0);

    let virt2 = VirtAddr::new_checked(0xffff_800f_0000_0023).unwrap();
    assert_eq!(0xffff_800f_0000_0023, virt2.0);

    let virt3 = VirtAddr::new_checked(0x0000_8000_0700_0000).unwrap();
    assert_eq!(0xffff_8000_0700_0000, virt3.0);

    assert!(VirtAddr::new_checked(0x1020_0000_0000_0002).is_err());
}