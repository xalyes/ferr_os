use core::ops::Range;

pub fn set_bit(num: &mut u64, n: u8, value: bool) {
    let mask = 1 << n;
    if value {
        *num |= mask;
    } else {
        *num &= !mask;
    }
}

pub fn set_bits(num: &mut u64, mask: u64, shift: u8) {
    let mask = mask << shift;

    *num |= mask;
}

pub fn get_bits(num: u64, range: Range<u8>) -> u64 {
    let cut_first_bits = num >> range.start;
    if range.end != 64 {
        cut_first_bits & (0xffff_ffff_ffff_ffff >> range.end)
    } else {
        cut_first_bits
    }
}

#[test_case]
fn set_bit_test() {
    let mut num = 0b1000_0000;

    set_bit(&mut num, 4, true);
    assert_eq!(0b1001_0000, num);

    set_bit(&mut num, 7, false);
    assert_eq!(0b0001_0000, num);
}

#[test_case]
fn set_bits_test() {
    let mask = 0b1001_1010;

    let mut num = 1 << 47;

    set_bits(&mut num, mask, 16);

    assert_eq!(0x8000_009a_0000, num);
}

#[test_case]
fn get_bits_test() {
    assert_eq!(0b101, get_bits(0b0010_1000, 3..6));
    assert_eq!(1, get_bits(0x8000_0000_0000_0000, 63..64));
    assert_eq!(0x3777, get_bits(0x0000_3777_0000_0000, 32..48));
    assert_eq!(0x22, get_bits(0x0000_0000_0000_0022, 0..6));
}