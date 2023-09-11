use alloc::alloc::{alloc, alloc_zeroed};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::error::Error;
use core::fmt::{Display, Formatter};
use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut, read_unaligned, read_volatile, slice_from_raw_parts, slice_from_raw_parts_mut, write_volatile};
use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::structures::paging::{OffsetPageTable, Translate};
use x86_64::VirtAddr;
use bitfield_struct::bitfield;
use crate::{pci, println, serial_println};
use crate::disk::DiskAccessError::{NoCommandSlots, TaskFileError};
use crate::pci::{DeviceConfigurationSpace, DeviceIdentification};

static PORTS: Mutex<Vec<AHCIPort>> = Mutex::new(Vec::new());

#[repr(u8)]
enum FISType {
    RegH2D = 0x27,
    RegD2H = 0x34,
    DMAActivate = 0x39,
    DMASetup = 0x41,
    Data = 0x46,
    BIST = 0x58,
    PIOSetup = 0x5F,
    DeviceBits = 0xA1,
}

#[repr(C, packed)]
struct FISRegH2D {
    fis_type: FISType,
    bit_field: FISRegH2DBitField,
    command: u8,
    feature_low: u8,
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,
    lba3: u8,
    lba4: u8,
    lba5: u8,
    feature_high: u8,
    count: u16,
    icc: u8,
    control: u8,
    _reserved1: [u8; 4],
}

#[bitfield(u8)]
struct FISRegH2DBitField {
    #[bits(4)] pmport: u8,
    #[bits(3)] _reserved0: u8,
    #[bits(1)] c: u8,
}

#[repr(C, packed)]
struct FISRegD2H {
    fis_type: FISType,
    bit_field: FISRegD2HBitField,
    status: u8,
    error: u8,
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,
    lba3: u8,
    lba4: u8,
    lba5: u8,
    _reserved2: u8,
    count: u16,
    _reserved3: u16,
    _reserved4: [u8; 4],
}

#[bitfield(u8)]
struct FISRegD2HBitField {
    #[bits(4)] pmport: u8,
    #[bits(2)] _reserved0: u8,
    #[bits(1)] interrupt: u8,
    #[bits(1)] _reserved1: u8,
}

#[repr(C, packed)]
struct FISData {
    fis_type: FISType,
    bitfield: FISDataBitField,
    _reserved1: [u8; 2],
    data: [u8]
}

#[bitfield(u8)]
struct FISDataBitField {
    #[bits(4)] pmport: u8,
    #[bits(4)] _reserved0: u8,
}

#[repr(C, packed)]
struct FISPIOSetup {
    fis_type: FISType,
    bit_field: FISPIOSetupBitField,
    status: u8,
    error: u8,
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,
    lba3: u8,
    lba4: u8,
    lba5: u8,
    _reserved2: u8,
    count: u16,
    _reserved3: u8,
    e_status: u8,
    transfer_count: u16,
    _reserved4: [u8; 2],
}

#[bitfield(u8)]
struct FISPIOSetupBitField {
    #[bits(4)] pmport: u8,
    #[bits(1)] _reserved0: u8,
    #[bits(1)] data_direction: u8,
    #[bits(1)] interrupt: u8,
    #[bits(1)] _reserved1: u8,
}

#[repr(C, packed)]
struct FISDMASetup {
    fis_type: FISType,
    bit_field: FISDMASetupBitfield,
    _reserved1: [u8; 2],
    dma_buffer_id: u64,
    _reserved2: [u8; 4],
    dma_buffer_offset: u32,
    transfer_count: u32,
    _reserved3: [u8; 4],
}

#[bitfield(u8)]
struct FISDMASetupBitfield {
    #[bits(4)] pmport: u8,
    #[bits(1)] _reserved0: u8,
    #[bits(1)] data_direction: u8,
    #[bits(1)] interrupt: u8,
    #[bits(1)] auto_activate: u8,
}

#[repr(C, packed)]
struct HBA {
    host_capability: u32,
    global_host_control: u32,
    interrupt_status: u32,
    port_implemented: u32,
    version: u32,
    ccc_control: u32,
    ccc_ports: u32,
    em_location: u32,
    em_control:  u32,
    host_capability_ex: u32,
    bios_handoff: u32,
    _reserved: [u8; 0xA0-0x2C],
    vendor: [u8; 0x100-0xA0],
    hba_ports: [HBAPort; 32],
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct HBAPort {
    cl_base: u32,
    cl_base_upper: u32,
    fis_base: u32,
    fis_base_upper: u32,
    interrupt_status: u32,
    interrupt_enable: u32,
    command_status: u32,
    _reserved0: [u8; 0x120 - 0x11C],
    task_file_data: u32,
    signature: u32,
    sata_status: u32,
    sata_control: u32,
    sata_error: u32,
    sata_active: u32,
    command_issue: u32,
    _reserved1: [u8; 0x180 - 0x13C],
}

#[repr(C, packed)]
struct HBAFIS {
    ds_fis: FISDMASetup,
    _pad0: [u8; 4],
    ps_fis: FISPIOSetup,
    _pad1: [u8; 12],
    r_fis: FISRegD2H,
    _pad2: [u8; 4],
    sdb_fis: [u8; 8], // TODO unknown structure should be here
    u_fis: [u8; 64],
    _reserved: [u8; 0x100-0xA0],
}

#[repr(C, packed)]
#[derive(Default, Copy, Clone)]
struct HBACommandHeader {
    bit_field0: HBACommandHeaderBitField0,
    bit_field1: HBACommandHeaderBitField1,
    prdt_length: u16,
    prd_byte_count: u32,
    command_table_descriptor_base: u64,
    _reserved: [u32; 4],
}

#[bitfield(u8)]
#[derive(Default)]
struct HBACommandHeaderBitField0 {
    #[bits(5)] command_fis_length: u8,
    #[bits(1)] atapi: u8,
    #[bits(1)] write: u8,
    #[bits(1)] prefetchable: u8,
}

#[bitfield(u8)]
#[derive(Default)]
struct HBACommandHeaderBitField1 {
    #[bits(1)] reset: u8,
    #[bits(1)] bist: u8,
    #[bits(1)] clear: u8,
    #[bits(1)] _reserved: u8,
    #[bits(4)] pmp: u8,
}

#[repr(C, packed)]
struct HBACommandTable {
    command_fis: [u8; 64],
    atapi_command: [u8; 16],
    _reserved: [u8; 48],
    prdt_entries: [HBAPRDTEntry; 8]
}

#[repr(C, packed)]
struct HBAPRDTEntry {
    data_base: u64,
    _reserved: u32,
    bit_field: HBAPRDTEntryBitField,
}

#[bitfield(u32)]
struct HBAPRDTEntryBitField {
    #[bits(22)] byte_count: usize,
    #[bits(9)] _reserved: usize,
    #[bits(1)] interrupt: usize,
}

struct AHCIPort {
    hba_mem: &'static HBA,
    port: &'static mut HBAPort,
    command_list_buffer: &'static mut [HBACommandHeader],
    fis_buffer: &'static mut [u8],
    command_table_buffers: Vec<&'static mut [u8]>,
}

#[derive(Debug)]
pub enum DiskAccessError {
    NoCommandSlots,
    TaskFileError,
}

impl Display for DiskAccessError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        let err_string = match self {
            NoCommandSlots => "no command slots",
            TaskFileError => "task file error",
        };
        write!(f, "{}", err_string)
    }
}

// TODO refactor into multiple functions
pub fn init() {
    let mapper = crate::MAPPER.get()
        .expect("mapper not initialized");
    let mut ports: Vec<AHCIPort> = Vec::new();

    let controllers: Vec<&&DeviceConfigurationSpace> = pci::DEVICES
        .get()
        .unwrap()
        .iter()
        .filter(|device_space| {
            let device = device_space.get_identification();
            device.class == 0x1
                && device.subclass == 0x6
                && device.interface == 0x1
        }).collect();
    for controller in controllers {
        init_device(mapper, &mut ports, controller);
    }
    *PORTS.lock() = ports;
}

fn init_device(
    mapper: &OffsetPageTable, ports: &mut Vec<AHCIPort>, controller: &DeviceConfigurationSpace
) {
    let bars = controller.get_bars();
    let abar_addr = bars[5];
    let mut hba_ptr = (abar_addr as u64 + mapper.phys_offset().as_u64()) as *mut HBA;

    // find devices
    for i in 0..32 {
        // check if port is implemented
        let bit = unsafe { (*hba_ptr).port_implemented & (1 << i) };
        if bit == 0 {
            continue;
        }

        let (command_list_buffer, fis_buffer, command_tables) = {
            let hba_mem = unsafe { &mut *(hba_ptr) };
            let port = &mut hba_mem.hba_ports[i];

            let sata_status = unsafe { read_volatile(addr_of!(port.sata_status)) };
            if (sata_status & 0x101) != 0x101
            {
                continue;
            }

            // initialize device
            // clear start and FIS receive enable
            let mut command_value = unsafe { read_volatile(addr_of!(port.command_status)) };
            command_value &= !0x0001;
            command_value &= !0x0010;
            unsafe { write_volatile(addr_of_mut!(port.command_status), command_value); }
            loop {
                let command_value = unsafe { read_volatile(addr_of!(port.command_status)) };
                if ((command_value & 0x4000) == 0) && ((command_value & 0x8000) == 0) {
                    break;
                }
            }

            // setup command list buffer
            let mut command_list_buffer: &'static mut [HBACommandHeader] = unsafe {
                let layout = Layout::from_size_align_unchecked(1024, 1024);
                let mem = alloc(layout);
                &mut *(slice_from_raw_parts_mut(mem as *mut HBACommandHeader, 32))
            };
            let phys_addr = mapper.translate_addr(
                VirtAddr::from_ptr(command_list_buffer.as_ptr())
            )
                .unwrap()
                .as_u64();
            unsafe {
                write_volatile(addr_of_mut!(port.cl_base), (phys_addr & u32::MAX as u64) as u32);
                write_volatile(addr_of_mut!(port.cl_base_upper), (phys_addr >> 32) as u32);
            }

            // setup fis buffer
            let fis_buffer: &'static mut [u8] = unsafe {
                let layout = Layout::from_size_align_unchecked(256, 256);
                let mem = alloc(layout);
                &mut *(slice_from_raw_parts_mut(mem, 256))
            };
            let phys_addr = mapper.translate_addr(
                VirtAddr::from_ptr(fis_buffer.as_ptr())
            )
                .unwrap()
                .as_u64();
            unsafe {
                write_volatile(addr_of_mut!(port.fis_base), (phys_addr & u32::MAX as u64) as u32);
                write_volatile(addr_of_mut!(port.fis_base_upper), (phys_addr >> 32) as u32);
            }

            let mut command_tables: Vec<&'static mut [u8]> = Vec::with_capacity(32);
            for (i, cmd_header) in command_list_buffer.iter_mut().enumerate() {
                let command_table: &'static mut [u8] = unsafe {
                    let layout = Layout::from_size_align_unchecked(32 * 256, 128);
                    let mem = alloc_zeroed(layout);
                    &mut *(slice_from_raw_parts_mut(mem, 32 * 256))
                };
                let phys_addr = mapper.translate_addr(
                    VirtAddr::from_ptr(command_table.as_ptr())
                )
                    .unwrap()
                    .as_u64();
                unsafe {
                    write_volatile(addr_of_mut!(cmd_header.command_table_descriptor_base), phys_addr);
                    write_volatile(addr_of_mut!(cmd_header.prdt_length), 8);
                }
                command_tables.push(command_table);
            }

            loop {
                let command_value = unsafe { read_volatile(addr_of!(port.command_status)) };
                if (command_value & 0x8000) == 0 {
                    break;
                }
            }

            unsafe {
                let mut command_value = read_volatile(addr_of!(port.command_status));
                command_value |= 0x01 | 0x100;
                write_volatile(addr_of_mut!(port.command_status), command_value);
            }

            (command_list_buffer, fis_buffer, command_tables)
        };

        let hba_mem = unsafe { &*hba_ptr };
        let ahci_port = AHCIPort {
            hba_mem,
            port: unsafe {&mut (*hba_ptr).hba_ports[i]}, //&hba_mem.hba_ports[i],
            command_list_buffer,
            fis_buffer,
            command_table_buffers: command_tables,
        };
        ports.push(ahci_port);
    }
}

pub fn read_sectors(
    mapper: &OffsetPageTable, drive_num: usize, lba: u64, sector_count: u16
) -> Result<Vec<u8>, DiskAccessError> {
    let ahci_port = &mut PORTS.lock()[drive_num];

    unsafe {
        write_volatile(addr_of_mut!(ahci_port.port.interrupt_status), u32::MAX);
    }

    let slot = find_command_slot(ahci_port).expect("No free command slots");

    // set command_fis_length and clear write bit
    let command_fis_length = size_of::<FISRegH2D>() / size_of::<u32>();
    ahci_port.command_list_buffer[slot].bit_field0.set_command_fis_length(command_fis_length as u8);
    ahci_port.command_list_buffer[slot].bit_field0.set_write(0);

    let cmd_table = unsafe {
        &mut *(ahci_port.command_table_buffers[slot].as_mut_ptr() as *mut HBACommandTable)
    };
    let mut prdt = &mut cmd_table.prdt_entries[0];

    let mut output_buffer = Vec::with_capacity(512 * sector_count as usize);
    unsafe { output_buffer.set_len(512 * sector_count as usize); }
    let output_buffer_phys_addr = mapper.translate_addr(
        VirtAddr::from_ptr(output_buffer.as_ptr())
        )
        .ok_or(NoCommandSlots)?
        .as_u64();

    unsafe {
        write_volatile(addr_of_mut!(prdt.data_base), output_buffer_phys_addr);
    }
    prdt.bit_field = prdt.bit_field.with_byte_count(512 * sector_count as usize - 1);
    prdt.bit_field = prdt.bit_field.with_interrupt(1);

    let cmd_fis = unsafe {
        &mut *(cmd_table.command_fis.as_ptr() as *mut FISRegH2D)
    };

    let lbas = lba.to_le_bytes();
    cmd_fis.fis_type = FISType::RegH2D;
    cmd_fis.bit_field.set_c(1);
    cmd_fis.command = 0x25;
    cmd_fis.lba0 = lbas[0];
    cmd_fis.lba1 = lbas[1];
    cmd_fis.lba2 = lbas[2];
    cmd_fis.lba3 = lbas[3];
    cmd_fis.lba4 = lbas[4];
    cmd_fis.lba5 = lbas[5];
    cmd_fis.device = 64;
    cmd_fis.count = sector_count;

    loop {
        let tfd = unsafe { read_volatile(addr_of!(ahci_port.port.task_file_data)) };
        if (tfd & 0x88) == 0 {
            break;
        }
    }

    unsafe {
        let mut command_issue = read_volatile(addr_of!(ahci_port.port.command_issue));
        command_issue |= 0b1 << slot;
        write_volatile(addr_of_mut!(ahci_port.port.command_issue), command_issue);
    }

    loop {
        let ci = unsafe { read_volatile(addr_of!(ahci_port.port.command_issue)) };
        if (ci & (0b1 << slot)) == 0 {
            break;
        }

        let is = unsafe { read_volatile(addr_of!(ahci_port.port.interrupt_status)) };
        if ((is >> 30) & 0b1) == 1 {
            return Err(TaskFileError)
        }
    }

    let is = unsafe { read_volatile(addr_of!(ahci_port.port.interrupt_status)) };
    if ((is >> 30) & 0b1) == 1 {
        return Err(TaskFileError)
    }

    Ok(output_buffer)
}

pub fn write_sectors(
    mapper: &OffsetPageTable, drive_num: usize, lba: u64, sector_count: u16, buffer: Vec<u8>,
) -> Result<(), DiskAccessError> {
    let ahci_port = &mut PORTS.lock()[drive_num];

    unsafe {
        write_volatile(addr_of_mut!(ahci_port.port.interrupt_status), u32::MAX);
    }

    let slot = find_command_slot(ahci_port).expect("No free command slots");

    // set command_fis_length and clear write bit
    let command_fis_length = size_of::<FISRegH2D>() / size_of::<u32>();
    ahci_port.command_list_buffer[slot].bit_field0.set_command_fis_length(command_fis_length as u8);
    ahci_port.command_list_buffer[slot].bit_field0.set_write(1);

    let cmd_table = unsafe {
        &mut *(ahci_port.command_table_buffers[slot].as_mut_ptr() as *mut HBACommandTable)
    };
    let mut prdt = &mut cmd_table.prdt_entries[0];

    //let mut output_buffer = Vec::with_capacity(512 * sector_count as usize);
    //unsafe { output_buffer.set_len(512 * sector_count as usize); }
    let output_buffer_phys_addr = mapper.translate_addr(
        VirtAddr::from_ptr(buffer.as_ptr())
    )
        .ok_or(NoCommandSlots)?
        .as_u64();

    unsafe {
        write_volatile(addr_of_mut!(prdt.data_base), output_buffer_phys_addr);
    }
    prdt.bit_field = prdt.bit_field.with_byte_count(512 * sector_count as usize - 1);
    prdt.bit_field = prdt.bit_field.with_interrupt(1);

    let cmd_fis = unsafe {
        &mut *(cmd_table.command_fis.as_ptr() as *mut FISRegH2D)
    };

    let lbas = lba.to_le_bytes();
    cmd_fis.fis_type = FISType::RegH2D;
    cmd_fis.bit_field.set_c(1);
    cmd_fis.command = 0x35;
    cmd_fis.lba0 = lbas[0];
    cmd_fis.lba1 = lbas[1];
    cmd_fis.lba2 = lbas[2];
    cmd_fis.lba3 = lbas[3];
    cmd_fis.lba4 = lbas[4];
    cmd_fis.lba5 = lbas[5];
    cmd_fis.device = 64;
    cmd_fis.count = sector_count;

    loop {
        let tfd = unsafe { read_volatile(addr_of!(ahci_port.port.task_file_data)) };
        if (tfd & 0x88) == 0 {
            break;
        }
    }

    unsafe {
        let mut command_issue = read_volatile(addr_of!(ahci_port.port.command_issue));
        command_issue |= 0b1 << slot;
        write_volatile(addr_of_mut!(ahci_port.port.command_issue), command_issue);
    }

    loop {
        let ci = unsafe { read_volatile(addr_of!(ahci_port.port.command_issue)) };
        if (ci & (0b1 << slot)) == 0 {
            break;
        }

        let is = unsafe { read_volatile(addr_of!(ahci_port.port.interrupt_status)) };
        if ((is >> 30) & 0b1) == 1 {
            return Err(TaskFileError)
        }
    }

    let is = unsafe { read_volatile(addr_of!(ahci_port.port.interrupt_status)) };
    if ((is >> 30) & 0b1) == 1 {
        return Err(TaskFileError)
    }

    Ok(())
}

/// tries to find an unused command slot index in a given ahci port
fn find_command_slot(ahci_port: &AHCIPort) -> Option<usize> {
    let slots = ahci_port.port.sata_active | ahci_port.port.command_issue;
    for i in 0..((ahci_port.hba_mem.host_capability >> 8) & 0xF0) {
        if ((slots >> i) & 0b1) == 0 {
            return Some(i as usize)
        }
    }
    None
}

pub fn get_disk_count() -> usize {
    PORTS.lock().len()
}