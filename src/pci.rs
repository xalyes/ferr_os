use crate::port::Port;

unsafe fn pci_config_read_word(bus: u8, device: u8, func: u8, offset: u8) -> u16 {
    let address: u32 =
        (bus as u32) << 16
        | (device as u32) << 11
        | (func as u32) << 8
        | (offset as u32 & 0xFC)
        | 0x80000000u32;

    let mut config_address_port = Port::new(0xCF8);
    config_address_port.write_u32(address);

    let mut config_data_port = Port::new(0xCFC);
    ((config_data_port.read_u32() >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

fn get_device_type(class_code: u8, subclass: u8, prog_if: u8) -> &'static str {
    if class_code == 0x6 && subclass == 0x0 {
        return "Host Bridge"
    } else if class_code == 0x6 && subclass == 0x1 {
        return "ISA Bridge"
    } else if class_code == 0x3 && subclass == 0x0 {
        return if prog_if == 0 {
            "VGA Controller"
        } else if prog_if == 1 {
            "VGA 8514-Compatible Controller"
        } else {
            "VGA Compatible Controller"
        }
    } else if class_code == 0x2 && subclass == 0x0 {
        return "Ethernet Controller"
    } else if class_code == 0x1 && subclass == 0x1 {
        return if prog_if == 0x0 {
            "IDE ISA Compatibility mode-only controller"
        } else if prog_if == 0x5 {
            "IDE PCI native mode-only controller"
        } else if prog_if == 0xA {
            "IDE ISA Compatibility mode controller, supports both channels switched to PCI native mode"
        } else if prog_if == 0xF {
            "IDE PCI native mode controller, supports both channels switched to ISA compatibility mode"
        } else if prog_if == 0x80 {
            "IDE ISA Compatibility mode-only controller, supports bus mastering"
        } else if prog_if == 0x85 {
            "IDE PCI native mode-only controller, supports bus mastering"
        } else if prog_if == 0x8A {
            "IDE ISA Compatibility mode controller, supports both channels switched to PCI native mode, supports bus mastering"
        } else if prog_if == 0x8F {
            "IDE PCI native mode controller, supports both channels switched to ISA compatibility mode, supports bus mastering"
        } else {
            "Some IDE Controller"
        }
    } else if class_code == 0x6 && subclass == 0x80 {
        return "Other bridge"
    }
    ""
}



unsafe fn check_function(bus: u8, device: u8, func: u8, vendor_id: u16) {
    let [_bist, header_type] = pci_config_read_word(bus, device, func, 0xE).to_be_bytes();
    let [class_code, subclass] = pci_config_read_word(bus, device, func, 0xA).to_be_bytes();
    let [prog_if, _revision_id] = pci_config_read_word(bus, device, func, 0x8).to_be_bytes();

    let device_type_str = get_device_type(class_code, subclass, prog_if);

    let mut prefix = "";
    if func != 0 {
        prefix = "|--- ";
    }

    if device_type_str == "" {
        log::info!("[pci] {}device #{} - vendor: {:#x}, header_type: {:#x}, class: {:#x}, subclass: {:#x}, func: {}", prefix, device, vendor_id, header_type, class_code, subclass, func);
    } else {
        log::info!("[pci] {}device #{} - vendor: {:#x}, header_type: {:#x}, func: {}, device_type: {}", prefix, device, vendor_id, header_type, func, device_type_str);
    }

    /*if class_code == 0x1 && subclass == 0x1 {
        crate::ide::ide_initialize(prog_if);
    }*/
}

unsafe fn check_device(bus: u8, device: u8) {
    let vendor_id = pci_config_read_word(bus, device, 0, 0);

    // device doesn't exist
    if vendor_id == 0xFFFF {
        return;
    }
    let [_bist, header_type] = pci_config_read_word(bus, device, 0, 0xE).to_be_bytes();

    if header_type & 0x80 != 0 {
        // it's a multifunction device!
        let [class_code, subclass] = pci_config_read_word(bus, device, 0, 0xA).to_be_bytes();
        log::info!("[pci] multifunction device #{} - vendor: {:#x}, header_type: {:#x}, device_type: {}", device, vendor_id, header_type, get_device_type(class_code, subclass, 0));

        for func in 1..8 {
            let vendor_id = pci_config_read_word(bus, device, func, 0);
            if vendor_id != 0xFFFF {
                check_function(bus, device, func, vendor_id);
            }
        }
        return;
    }

    check_function(bus, device, 0, vendor_id);
}

pub fn init_pci() {

    unsafe {
        let [_bist, header_type] = pci_config_read_word(0, 0, 0, 0xE).to_be_bytes();

        if header_type & 0x80 == 0 {
            // Single PCI host controller
            // checking bus #0
            for device in 0..32 {
                check_device(0, device);
            }
        } else {
            // Multiple PCI host controllers
            unimplemented!();
        }
    }
}