use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::cmp::min;
use crate::ide::BlockDevice;

#[repr(C,packed)]
struct PartitionTableEntry {
    bootable: u8,
    starting_chs: [u8; 3],
    partition_type: u8,
    ending_chs: [u8; 3],
    starting_lba: u32,
    ending_lba: u32,
}

#[repr(C,packed)]
struct ProtectiveMasterBootRecord {
    mbr_bootstrap: [u8; 440],
    signature: u32,
    reserved: u16,
    entry_0: PartitionTableEntry,
    entry_1: PartitionTableEntry,
    entry_2: PartitionTableEntry,
    entry_3: PartitionTableEntry,
    valid_bootsector_signature: [u8; 2],
}

#[repr(C,packed)]
struct PartitionTableHeader {
    signature: [u8; 8], // should be 'EFI PART'
    gpt_revision: u32,
    header_size: u32,
    header_checksum: u32,
    reserved: u32,
    this_header_lba: u64,
    alternate_header_lba: u64,
    first_usable_block: u64,
    last_usable_block: u64,
    disk_guid: u128,
    starting_lba_of_array: u64,
    entries_num: u32,
    entry_size: u32,
    array_checksum: u32,
    reserved_tail: [u8; 420],
}

#[repr(C, packed)]
struct PartitionEntry {
    partition_type_guid: u128, // zero is unused entry
    unique_partition_guid: u128,
    starting_lba: u64,
    ending_lba: u64,
    attributes: u64,

    // partition name is usually 72 bytes of UTF-16LE
    // but it can be changed in the header and his 'entry_size' field
    partition_name_and_tail: [u8; 456],
}

#[derive(Debug)]
pub enum GptError {
    InvalidProtectiveMBR,
    InvalidPartitionTableHeader,
    InvalidTableHeaderChecksum,
    InvalidMyLbaHeader,
    InvalidEntriesArrayChecksum,
}

pub fn guid_to_str(guid: u128) -> String {
    let slice = guid.to_le_bytes();
    format!("{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            slice[3], slice[2], slice[1], slice[0],
            slice[5], slice[4],
            slice[7], slice[6],
            slice[8], slice[9],
            slice[10], slice[11], slice[12], slice[13], slice[14], slice[15])
}

pub fn parse_gpt(device: Box<dyn BlockDevice>) -> Result<(), GptError> {
    log::info!("[gpt] Parsing GPT for {}kb block {:?} device on channel {:?}", (device.size() * 512) / 1024, device.drive_type(), device.channel());

    let lba0 = device.read(0x0, 1).expect("Failed to read LBA 0")[0];

    let protective_mbr = lba0.as_ptr() as *const ProtectiveMasterBootRecord;

    if unsafe { (*protective_mbr).valid_bootsector_signature } != [0x55, 0xaa] {
        return Err(GptError::InvalidProtectiveMBR);
    }

    let first_partition_mbr = unsafe { &protective_mbr.as_ref().unwrap().entry_0 };

    if first_partition_mbr.bootable != 0x0
        || first_partition_mbr.starting_chs != [0x0, 0x2, 0x0]
        || first_partition_mbr.partition_type != 0xee
        || first_partition_mbr.starting_lba != 0x1 {
        return Err(GptError::InvalidProtectiveMBR)
    }

    let mut lba1 = device.read(0x1, 1).expect("Failed to read LBA 1")[0];

    let partition_table_header = unsafe { (lba1.as_mut_ptr() as *mut PartitionTableHeader).as_mut().unwrap() };

    // we expect 'EFI PART'
    if partition_table_header.signature != [0x45, 0x46, 0x49, 0x20, 0x50, 0x41, 0x52, 0x54] {
        return Err(GptError::InvalidPartitionTableHeader);
    }

    let gpt_revision = partition_table_header.gpt_revision;
    let header_size = partition_table_header.header_size;
    let disk_guid = partition_table_header.disk_guid;
    let entries_num = partition_table_header.entries_num;
    let entry_size = partition_table_header.entry_size;
    let first_usable_lba = partition_table_header.first_usable_block;
    let last_usable_lba = partition_table_header.last_usable_block;

    log::info!("[gpt] GPT info: gpt revision: {:#x}, header size: {}, guid: {}, total entries: {}, size of entry: {}, usable LBAs {} - {}",
    gpt_revision,
    header_size,
    guid_to_str(disk_guid),
    entries_num,
    entry_size,
    first_usable_lba,
    last_usable_lba);

    let header_crc32 = partition_table_header.header_checksum;

    partition_table_header.header_checksum = 0;

    let header_bytes_slice = unsafe {
        core::slice::from_raw_parts(lba1.as_ptr().cast::<u8>(), header_size as usize)
    };

    if shared_lib::crc::calculate_crc32(header_bytes_slice) != header_crc32 {
        return Err(GptError::InvalidTableHeaderChecksum);
    }

    if partition_table_header.this_header_lba != 1 {
        return Err(GptError::InvalidMyLbaHeader);
    }

    let entries_lba = device.read(partition_table_header.starting_lba_of_array as u32,
                                  ((partition_table_header.entries_num * partition_table_header.entry_size) / 512) as u8)
        .expect("Failed to read LBAs of partition entry array");

    let mut entries_checksum = 0xFFFFFFFF;
    let mut bytes_remain = partition_table_header.entries_num * partition_table_header.entry_size;

    for entry_lba in &entries_lba {
        let entry_slice = unsafe {
            core::slice::from_raw_parts(entry_lba.as_ptr().cast::<u8>(), min(bytes_remain as usize, 512))
        };
        entries_checksum = shared_lib::crc::calculate_crc32_partial(entry_slice, entries_checksum);
        if bytes_remain > 512 {
            bytes_remain -= 512;
        }
    }

    if partition_table_header.array_checksum != !entries_checksum {
        return Err(GptError::InvalidEntriesArrayChecksum);
    }

    for (idx, entry_lba) in entries_lba.iter().enumerate() {
        for i in 0..(512 / partition_table_header.entry_size) {
            let partition_entry = unsafe {
                (entry_lba.as_ptr().offset((i * partition_table_header.entry_size / 2) as isize) as *const PartitionEntry).as_ref().unwrap()
            };
            let partition_type_guid = partition_entry.partition_type_guid;
            if partition_type_guid == 0 { // unused entry
                continue;
            }

            let unique_partition_guid = partition_entry.unique_partition_guid;
            let starting_lba = partition_entry.starting_lba;
            let ending_lba = partition_entry.ending_lba;
            let attributes = partition_entry.attributes;
            let partition_name = partition_entry.partition_name_and_tail.split_at((partition_table_header.entry_size - 0x38 + 1) as usize).0;

            log::info!("[gpt] entry at LBA {}:{} - type: {}, id: {} [{}-{}] {} {}", idx + partition_table_header.starting_lba_of_array as usize,
                i, guid_to_str(partition_type_guid), guid_to_str(unique_partition_guid), starting_lba, ending_lba,
                attributes, core::str::from_utf8(partition_name).unwrap());
        }
    }

    log::info!("[gpt] Parsing ok");
    return Ok(())
}