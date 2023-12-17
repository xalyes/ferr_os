
pub fn set_bit(num: &mut u64, n: u8, value: bool) {
    let mask = (value as u64) << n;

    *num |= mask;
}

pub fn set_bits(num: &mut u64, mask: u64, shift: u8) {
    let mask = mask << shift;

    *num |= mask;
}

#[test_case]
fn set_bit_test() {
    let mut num = 0b1000_0000;

    set_bit(&mut num, 5, true);
    set_bit(&mut num, 7, false);

    assert_eq!(0b0001_0000, num);
}