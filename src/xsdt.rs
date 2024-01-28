use core::slice::from_raw_parts;
use shared_lib::addr::{PhysAddr, VirtAddr};
use shared_lib::frame_allocator::FrameAllocator;
use shared_lib::page_table::{align_down, align_down_u64, map_address_with_offset};
use shared_lib::VIRT_MAPPING_OFFSET;
use crate::memory::active_level_4_table;

#[repr(C)]
struct RsdpV2 {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oemid: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32, // deprecated

    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

fn wrapping_sum(arr: &[u8]) -> u8 {
    arr.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

fn get_xsdt_address(rsdp_addr: PhysAddr) -> VirtAddr {
    let rsdp_virt_addr = {
        log::info!("RSDP: {:#x}", rsdp_addr.0);
        let rsdp_virt_addr = VirtAddr::new_checked(rsdp_addr.0 + VIRT_MAPPING_OFFSET).unwrap();

        log::info!("RSDP virt - {:#x}", rsdp_virt_addr.0);
        rsdp_virt_addr
    };

    let acpi_revision_ptr = rsdp_virt_addr.offset(8 + 1 + 6).unwrap().0 as *const u8;
    let acpi_revision = unsafe { *acpi_revision_ptr };

    log::info!("ACPI revision: {}", acpi_revision);
    if acpi_revision != 2 {
        panic!("ACPI1 is not supported!");
    }

    let rsdp_ptr = rsdp_virt_addr.0 as *mut RsdpV2;
    let rsdp = unsafe { rsdp_ptr.as_mut().unwrap() };

    let v1_bytes_sum = wrapping_sum(&rsdp.signature)
        + rsdp.checksum
        + wrapping_sum(&rsdp.oemid)
        + rsdp.revision
        + wrapping_sum(&rsdp.rsdt_address.to_ne_bytes());
    let v2_bytes_sum = wrapping_sum(&rsdp.length.to_ne_bytes())
        + wrapping_sum(&rsdp.xsdt_address.to_ne_bytes())
        + rsdp.extended_checksum
        + wrapping_sum(&rsdp.reserved);

    log::info!("v1_bytes_sum: {:#x}", v1_bytes_sum);
    log::info!("v2_bytes_sum: {:#x}", v2_bytes_sum);

    if v1_bytes_sum != 0 {
        panic!("ACPI1 checksum failed");
    }

    if v2_bytes_sum != 0 {
        panic!("ACPI2 checksum failed");
    }
    VirtAddr::new_checked(rsdp.xsdt_address + VIRT_MAPPING_OFFSET).unwrap()
}

#[repr(C)]
struct AcpiSdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oemid: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32
}

impl AcpiSdtHeader {
    pub fn get_bytes_sum(&self) -> u8 {
        wrapping_sum(&self.signature)
            .wrapping_add(wrapping_sum(&self.creator_id.to_ne_bytes()))
            .wrapping_add(wrapping_sum(&self.length.to_ne_bytes()))
            .wrapping_add(wrapping_sum(&self.oemid))
            .wrapping_add(self.revision)
            .wrapping_add(wrapping_sum(&self.creator_revision.to_ne_bytes()))
            .wrapping_add(wrapping_sum(&self.oem_revision.to_ne_bytes()))
            .wrapping_add(self.checksum)
            .wrapping_add(wrapping_sum(&self.oem_table_id))
    }
}

#[repr(C)]
struct MadtHeader {
    pub local_apic_addr: u32,
    pub apic_flags: u32,
}

#[repr(C)]
struct MadtEntryHeader {
    pub entry_type: u8,
    pub record_length: u8
}

#[repr(C)]
struct MadtEntryIOApic {
    pub io_apic_id: u8,
    pub reserved: u8,
    pub io_apic_addr: u32,
    pub global_system_interrupt_base: u32
}

#[repr(C)]
struct MadtEntryIOApicInterruptSource {
    pub bus_source: u8,
    pub irq_source: u8,
    pub global_system_interrupt: u32,
    pub flags: u16
}

struct ApicPhysAddrs {
    pub local_apic_addr: PhysAddr,
    pub io_apic_addr: PhysAddr
}

fn handle_madt(header: &AcpiSdtHeader, data_addr: VirtAddr) -> Result<ApicPhysAddrs, &'static str> {
    log::info!("MADT handling. Len: {}", header.length);

    let mut check_sum = header.get_bytes_sum();
    for i in 0..(header.length - 36) {
        unsafe {
            check_sum = check_sum.wrapping_add(*((data_addr.0 + i as u64) as *const u8));
        }
    }
    if check_sum != 0 {
        return Err("MADT checksum failed");
    }

    let madt_header = unsafe {
        (data_addr.0 as *const MadtHeader).as_ref().unwrap()
    };

    log::info!("local apic phys: {:#x} flags: {}", madt_header.local_apic_addr, madt_header.apic_flags);

    let mut result: Result<ApicPhysAddrs, &'static str> = Err("Invalid MADT");
    let mut offset: u64 = 8;
    while offset < (header.length - 36) as u64 {
        let entry_header = unsafe {
            ((data_addr.0 + offset) as *const MadtEntryHeader).as_ref().unwrap()
        };

        log::info!("MADT entry: type: {}, len: {}", entry_header.entry_type, entry_header.record_length);

        if entry_header.entry_type == 1 {
            let io_apic_entry = unsafe {
                ((data_addr.0 + offset + 2) as *const MadtEntryIOApic).as_ref().unwrap()
            };

            log::info!("io apic: addr: {:#x}, global system int base: {:#x}. id: {}", io_apic_entry.io_apic_addr, io_apic_entry.global_system_interrupt_base, io_apic_entry.io_apic_id);

            result = Ok(ApicPhysAddrs {
                local_apic_addr: PhysAddr(madt_header.local_apic_addr as u64),
                io_apic_addr: PhysAddr(io_apic_entry.io_apic_addr as u64)
            });
        } else if entry_header.entry_type == 2 {
            let io_apic_source_interrupt_entry = unsafe {
                ((data_addr.0 + offset + 2) as *const MadtEntryIOApicInterruptSource).as_ref().unwrap()
            };

            log::info!("Entry Type 2: I/O APIC Interrupt Source Override. {:#x} {:#x} {:#x} {:#x}", io_apic_source_interrupt_entry.bus_source, io_apic_source_interrupt_entry.irq_source, io_apic_source_interrupt_entry.global_system_interrupt, io_apic_source_interrupt_entry.flags);
        }

        offset += entry_header.record_length as u64;
    }
    result
}

pub struct ApicAddresses {
    pub local_apic_addr: VirtAddr,
    pub io_apic_addr: VirtAddr
}

pub fn read_xsdt(allocator: &mut FrameAllocator, rsdp_addr: u64) -> ApicAddresses {
    let xsdt_addr = get_xsdt_address(PhysAddr(rsdp_addr));
    log::info!("XSDT addr: {:#x}", xsdt_addr.0);

    let xsdt_header_ptr = xsdt_addr.0 as *mut AcpiSdtHeader;
    let xsdt_header = unsafe { xsdt_header_ptr.as_mut().unwrap() };
    log::info!("XSDT header: s:{:?}, len:{:#x}, rev:{}, ch: {}, oemid: {:?}, cr_rev: {:#x}", xsdt_header.signature, xsdt_header.length, xsdt_header.revision, xsdt_header.checksum,
    xsdt_header.oemid, xsdt_header.creator_revision);

    let mut check_sum = xsdt_header.get_bytes_sum();

    let pointers_to_other_sdts = unsafe { from_raw_parts((xsdt_addr.0 + 36) as *const u64, ((xsdt_header.length - 36) / 8) as usize) };

    for ptr in pointers_to_other_sdts {
        check_sum += wrapping_sum(&ptr.to_ne_bytes());
    }

    if check_sum != 0 {
        panic!("XSDT checksum failed");
    }

    let mut apic_addrs = ApicPhysAddrs { local_apic_addr:PhysAddr(0), io_apic_addr:PhysAddr(0)};
    for sdt_ptr in pointers_to_other_sdts {
        let header_ptr = (sdt_ptr + VIRT_MAPPING_OFFSET) as *const AcpiSdtHeader;
        let header = unsafe { header_ptr.as_ref().unwrap() };
        let s = match core::str::from_utf8(&header.signature) {
            Ok(v) => v,
            Err(e) => panic!("Invalid UTF-8 sequence: {}", e),
        };
        log::info!("Found SDT {}", s);
        if s == "APIC" {
            apic_addrs = handle_madt(header, VirtAddr::new_checked(sdt_ptr + VIRT_MAPPING_OFFSET + 36).unwrap()).unwrap();
        }
    }

    if apic_addrs.local_apic_addr.0 == 0 {
        panic!("Failed to find local APIC");
    }

    let mut apic_phys = apic_addrs.local_apic_addr.0;
    let mut apic_virt = VirtAddr::new(apic_addrs.local_apic_addr.0 + VIRT_MAPPING_OFFSET);
    let apic_virt_end = apic_virt.offset(0x10_0000)
        .expect("Failed to offset virtual address");

    let l4_table = unsafe {
        active_level_4_table()
    };

    while apic_virt < apic_virt_end {
        unsafe {
            map_address_with_offset(l4_table, apic_virt, apic_phys, allocator, VIRT_MAPPING_OFFSET)
                .expect("Failed to map new frame");
        }

        apic_virt = apic_virt.offset(4096).unwrap();
        apic_phys += 4096;
    }

    let io_apic_phys = apic_addrs.io_apic_addr.0 << 16; // hack. For some reason on qemu we need it
    let io_apic_virt = VirtAddr::new(io_apic_phys + VIRT_MAPPING_OFFSET);

    unsafe {
        map_address_with_offset(l4_table, align_down(io_apic_virt), align_down_u64(io_apic_phys), allocator, VIRT_MAPPING_OFFSET)
            .expect("Failed to map new frame");
    }

    ApicAddresses {
        local_apic_addr: VirtAddr::new_checked(apic_addrs.local_apic_addr.0 + VIRT_MAPPING_OFFSET).unwrap(),
        io_apic_addr: io_apic_virt
    }
}
