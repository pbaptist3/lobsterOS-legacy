use alloc::vec::Vec;
use core::mem::{size_of, transmute};
use core::ptr::{addr_of_mut, slice_from_raw_parts};
use conquer_once::spin::OnceCell;
use bitfield_struct::bitfield;
use crate::{acpi, println, serial_println};
use crate::acpi::SDTHeader;
use crate::pci::DeviceType::{Device, PCIBridge, CardBusBridge};

static MCFG: OnceCell<MCFGTable> = OnceCell::uninit();
pub static DEVICES: OnceCell<Vec<&'static DeviceConfigurationSpace>> = OnceCell::uninit();

#[derive(Debug, Copy, Clone)]
struct MCFGTable {
    header: &'static SDTHeader,
    entries: &'static [MCFGEntry],
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
struct MCFGEntry {
    config_space: u64,
    segment_group: u16,
    start_bus: u8,
    end_bus: u8,
    reserved: u32,
}

#[bitfield(u16)]
pub struct CommandRegister {
    #[bits(1)] io_space: u16,
    #[bits(1)] memory_space: u16,
    #[bits(1)] bus_master: u16,
    #[bits(1)] special_cycles: u16,
    #[bits(1)] memory_write: u16,
    #[bits(1)] vga_snoop: u16,
    #[bits(1)] parity_error_response: u16,
    #[bits(1)] _reserved0: u16,
    #[bits(1)] serr_enable: u16,
    #[bits(1)] fast_b2b_enable: u16,
    #[bits(1)] interrupt_disable: u16,
    #[bits(5)] _reserved1: u16,
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub struct DeviceConfigurationSpace {
    vendor_id: u16,
    device_id: u16,
    command: CommandRegister,
    status: u16,
    revision_id: u8,
    prog_if: u8,
    subclass: u8,
    class_code: u8,
    cache_line_size: u8,
    latency_timer: u8,
    header_type: u8,
    bist: u8,
    bar_0: u32,
    bar_1: u32,
    bar_2: u32,
    bar_3: u32,
    bar_4: u32,
    bar_5: u32,
    cis_ptr: u32,
    subsystem_vendor_id: u16,
    subsystem_id: u16,
    expansion_rom_base: u32,
    capabilities_ptr: u8,
    _reserved: [u8; 7],
    interrupt_line: u8,
    interrupt_pin: u8,
    min_grant: u8,
    max_latency: u8,
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct BaseConfigurationSpace {
    vendor_id: u16,
    device_id: u16,
    command: u16,
    status: u16,
    revision_id: u8,
    prog_if: u8,
    subclass: u8,
    class_code: u8,
    cache_line_size: u8,
    latency_timer: u8,
    header_type: u8,
    bist: u8,
}

// TODO add PCIBridge and CardBusBridge types
enum DeviceType {
    Device(&'static DeviceConfigurationSpace),
    PCIBridge(()),
    CardBusBridge(()),
}

pub struct DeviceIdentification {
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
}

impl DeviceType {
    /// Returns a device type enum containing a reference to the correct config space type
    /// requires addr to be a valid virt addr of a configuration space
    pub unsafe fn from(addr: u64) -> Option<Self> {
        let base_space = &*(addr as *const BaseConfigurationSpace);
        let header_type = base_space.header_type;
        match header_type & 0x7F {
            0x0 => Some(Device(transmute(base_space))),
            0x1 => Some(PCIBridge(())),
            0x2 => Some(CardBusBridge(())),
            0x7F => None,
            _ => panic!("Invalid header type detected: {}", header_type)
        }
    }

    pub fn as_device(&self) -> Option<&'static DeviceConfigurationSpace> {
        match self {
            DeviceType::Device(config_space) => Some(config_space),
            _ => None
        }
    }

    pub fn as_pci_bridge(&self) -> Option<&()> {
        match self {
            DeviceType::PCIBridge(config_space) => Some(config_space),
            _ => None
        }
    }

    pub fn as_cardbus_bridge(&self) -> Option<&()> {
        match self {
            DeviceType::CardBusBridge(config_space) => Some(config_space),
            _ => None
        }
    }
}

impl MCFGTable {
    pub fn from_header(header: &'static SDTHeader) -> Self {
        let header_start_addr = header as *const SDTHeader as u64;
        let header_end_addr = header_start_addr + size_of::<SDTHeader>() as u64;
        let entries_start_addr = header_end_addr + 8;
        let entry_count = (header.get_length() - size_of::<SDTHeader>() as u32 - 8)
            / size_of::<MCFGEntry>() as u32;
        let entries = unsafe {
            &*slice_from_raw_parts(
                entries_start_addr as *const MCFGEntry,
                entry_count as usize,
            )
        };

        Self {
            header,
            entries,
        }
    }

    pub fn get_entries(&self) -> &'static [MCFGEntry] {
        &self.entries
    }
}

impl DeviceConfigurationSpace {
    /// gets reference to device config space from a virt addr
    /// caller must guarantee address points to a device config space
    pub unsafe fn from_addr(addr: u64) -> &'static Self {
        &*(addr as *const DeviceConfigurationSpace)
    }

    pub fn get_identification(&self) -> DeviceIdentification {
        DeviceIdentification {
            class: self.class_code,
            subclass: self.subclass,
            interface: self.prog_if,
        }
    }

    /// returns UNALIGNED raw pointer to command register
    pub fn get_command_reg_mut(&mut self) -> *mut CommandRegister {
        addr_of_mut!(self.command)
    }

    pub fn get_bars(&self) -> [u32; 6] {
        [self.bar_0, self.bar_1, self.bar_2, self.bar_3, self.bar_4, self.bar_5]
    }
}

fn get_mcfg() -> MCFGTable {
    let mcfg_header = acpi::RSDT.get()
        .unwrap()
        .get_tables()
        .iter()
        .find(|sdt| sdt.get_signature_str() == "MCFG")
        .expect("No MCFG found in RSDT");
    let mcfg = MCFGTable::from_header(mcfg_header);
    mcfg
}

fn enumerate_bus(phys_offset: u64) -> Vec<&'static DeviceConfigurationSpace> {
    let mut devices: Vec<&'static DeviceConfigurationSpace> = Vec::new();

    let mcfg = MCFG.get().unwrap();
    for bridge in mcfg.entries {
        for bus in bridge.start_bus..bridge.end_bus {
            for device in 0..32 {
                unsafe { check_device(&mut devices, bridge, phys_offset, bus, device); }
            }
        }
    }

    devices
}

/// checks all functions of a device
unsafe fn check_device(
    devices: &mut Vec<&'static DeviceConfigurationSpace>,
    bridge: &MCFGEntry, phys_offset: u64, bus: u8, device: u8
) {
    let function = 0;
    let config_addr = phys_offset
        + bridge.config_space
        + (((bus as u64) << 20) | ((device as u64) << 15) | ((function as u64) << 12));
    let config_device = unsafe {
        match DeviceType::from(config_addr) {
            Some(device) => device,
            None => return
        }
    };


    // TODO add proper handling for all device types
    if config_device.as_device().is_none() {
        return;
    }
    let config_space = config_device.as_device().unwrap();

    // device does not exist
    if config_space.vendor_id == 0xFFFF {
        return;
    }

    check_function(devices, config_space);

    // multi-function device
    if (config_space.header_type & 0x80) != 0 {
        for function in 1..8 {
            let config_addr = phys_offset
                + bridge.config_space
                + ((bus as u64) << 20 | (device as u64) << 15 | (function as u64) << 12);
            let config_space = unsafe {
                DeviceConfigurationSpace::from_addr(config_addr)
            };
            // device exists
            if config_space.vendor_id != 0xFFFF {
                check_function(devices, config_space);
            }
        }
    }
}

fn check_function(
    devices: &mut Vec<&'static DeviceConfigurationSpace>,
    config_space: &'static DeviceConfigurationSpace
) {
    devices.push(config_space);
}

/// finds mcfg and enumerates pci bus
pub fn init(phys_offset: u64) {
    MCFG.init_once(|| get_mcfg());
    DEVICES.init_once(|| enumerate_bus(phys_offset))
}