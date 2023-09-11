use alloc::vec::Vec;
use core::mem::{size_of, size_of_val, transmute};
use core::ops::RangeInclusive;
use core::ptr::{null, slice_from_raw_parts};
use core::slice::Iter;
use conquer_once::spin::OnceCell;
use lazy_static::lazy_static;
use x86_64::VirtAddr;
use crate::{println, serial_println};

const RSDP_REGION: RangeInclusive<u64> = 0x80000..=0xFFFFF;

pub static RSDP: OnceCell<&'static RSDPDescriptor> = OnceCell::uninit();
pub static RSDT: OnceCell<RSDT> = OnceCell::uninit();

#[repr(C, packed)]
pub struct RSDPDescriptor {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

#[repr(C, packed)]
pub struct RSDPExtended {
    base_descriptor: RSDPDescriptor,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
pub struct SDTHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

pub struct RSDT {
    header: &'static SDTHeader,
    tables: Vec<&'static SDTHeader>,
}

impl RSDPDescriptor {
    fn verify_checksum(&self) -> bool {
        let bytes = unsafe { &*(self as *const Self as *const [u8; size_of::<Self>()]) };
        let mut checksum: u32 = 0;
        for byte in bytes {
            checksum += *byte as u32;
        }
        (checksum & 0xFF) == 0
    }
}

impl SDTHeader {
    pub fn get_signature_str(&self) -> &str {
        core::str::from_utf8(&self.signature).expect("SDT has invalid signature")
    }

    pub fn get_length(&self) -> u32 {
        self.length
    }
}

impl RSDT {
    pub fn from_header(header: &'static SDTHeader, physical_offset: u64) -> Self {
        let header_start_addr = header as *const SDTHeader as u64;
        let header_end_addr = header_start_addr + size_of::<SDTHeader>() as u64;
        let tables_count = (header.get_length() - size_of::<SDTHeader>() as u32) / 4;
        let tables = unsafe {
            let ptrs = &*slice_from_raw_parts(
                header_end_addr as *const u32,
                tables_count as usize,
            );
            let tables = ptrs
                .iter()
                .map(|table| {
                    &*((*table as u64 + physical_offset) as *const SDTHeader)
                })
                .collect();
            tables
        };

        Self {
            header,
            tables,
        }
    }

    pub fn get_tables(&self) -> &Vec<&'static SDTHeader> {
        &self.tables
    }
}

/// finds the RSDP (only works on BIOS and assumed ACPI revision 0)
fn find_rsdp(physical_offset: u64) -> Option<&'static RSDPDescriptor> {
    for phys_addr in RSDP_REGION.step_by(0x10) {
        let addr = physical_offset + phys_addr;
        let id_str_bytes = unsafe {
            &*(addr as *const [u8; 8])
        };
        if id_str_bytes == b"RSD PTR " {
            let potential_rsdp = unsafe {&*(addr as *const RSDPDescriptor)};
            if potential_rsdp.verify_checksum() {
                // TODO add logic for extended RSDP
                assert_eq!(potential_rsdp.revision, 0);
                return Some(potential_rsdp);
            }
        }
    }
    None
}

fn get_rsdt(physical_offset: u64) -> RSDT {
    let rsdp = RSDP.get().expect("No RSDP obtained yet");
    let rsdt_addr = rsdp.rsdt_address;
    let rsdt_header = unsafe {
        &*((rsdt_addr as u64 + physical_offset) as *const SDTHeader)
    };
    let rsdt = RSDT::from_header(rsdt_header, physical_offset);
    rsdt
}

pub fn init(phys_offset: u64) {
    let rsdp = find_rsdp(phys_offset)
        .expect("Failed to find RSDP signature in specified memory region");
    RSDP.init_once(|| rsdp);
    let rsdt = get_rsdt(phys_offset);
    RSDT.init_once(|| rsdt);
}