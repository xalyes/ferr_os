#![no_std]

pub use crate::boot_info::BootInfo;
pub use crate::logger::Logger;

/// Contains the boot information struct sent by the bootloader to the kernel on startup.
pub mod boot_info;
pub mod logger;