use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::error::Error;
use core::mem::size_of;
use core::mem::transmute;
use core::pin::Pin;
use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::structures::paging::OffsetPageTable;
use crate::disk::{DiskAccessError, read_sectors, write_sectors};
use crate::{println, serial_println};
use trees::{Tree, Node, TreeWalk};
use trees::walk::Visit;
use bitfield_struct::bitfield;

pub static FILE_SYSTEM: Mutex<Option<FileSystem>> = Mutex::new(None);
static BOOT_SECTOR: OnceCell<(usize, FatBootSector)> = OnceCell::uninit();

pub struct FileSystem(Tree<File>);

#[repr(C, packed)]
#[derive(Debug)]
pub struct FatBootSector {
    boot_jmp: [u8; 3],
    oem_name: [u8; 8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sector_count: u16,
    table_count: u8,
    root_entry_count: u16,
    total_sectors_16: u16,
    media_type: u8,
    table_size_16: u16,
    sectors_per_track: u16,
    head_side_count: u16,
    hidden_sector_count: u32,
    total_sectors_32: u32,
    extended_section: FatBootSector32Ext,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct FatBootSector32Ext {
    table_size_32: u32,
    extended_flags: u16,
    fat_version: u16,
    root_cluster: u32,
    fat_info: u16,
    backup_boot_sector: u16,
    _reserved_0: [u8; 12],
    drive_number: u8,
    _reserved_1: u8,
    boot_signature: u8,
    volume_id: u32,
    volume_label: [u8; 11],
    fat_type_label: [u8; 8],
}

#[bitfield(u8)]
#[derive(Default)]
pub struct FileAttributes {
    #[bits(1)] read_only: usize,
    #[bits(1)] hidden: usize,
    #[bits(1)] system: usize,
    #[bits(1)] volume_id: usize,
    #[bits(1)] directory: usize,
    #[bits(1)] archive: usize,
    #[bits(2)] _reserved: usize,
}

#[bitfield(u16)]
#[derive(Default)]
pub struct FileTime {
    #[bits(5)] seconds: usize,
    #[bits(6)] minutes: usize,
    #[bits(5)] hours: usize,
}

#[bitfield(u16)]
#[derive(Default)]
pub struct FileDate {
    #[bits(5)] day: usize,
    #[bits(4)] month: usize,
    #[bits(7)] year: usize,
}

#[repr(C, packed)]
#[derive(Debug, Default, Copy, Clone)]
pub struct File {
    name: [u8; 11],
    attributes: FileAttributes,
    _reserved: u8,
    creation_duration: u8,
    creation_time: FileTime,
    creation_date: FileDate,
    last_accessed: FileDate,
    cluster_h: u16,
    modified_time: FileTime,
    modified_date: FileDate,
    cluster_l: u16,
    file_size: u32,
}

impl File {
    pub fn get_name(&self) -> &str {
        if self.name[0] == 0 {
            "/"
        } else {
            let mut name = core::str::from_utf8(&self.name)
                .expect("invalid ascii in filename");
            name
        }
    }

    pub fn get_data_addr(&self) -> u32 {
        self.cluster_l as u32 | ((self.cluster_h as u32) << 16)
    }

    pub fn get_data(
        &self, mapper: &OffsetPageTable
    ) -> Result<Vec<u8>, DiskAccessError> {
        let (drive_num, boot_sector) = BOOT_SECTOR.get()
            .expect("no boot sector initialized");
        let fat_size = if boot_sector.table_size_16 == 0 {
            boot_sector.extended_section.table_size_32
        } else {
            boot_sector.table_size_16 as u32
        };
        let root_dir_sectors = (boot_sector.root_entry_count*32 + boot_sector.bytes_per_sector-1)
            / boot_sector.bytes_per_sector;
        let first_data_sector = boot_sector.reserved_sector_count as u32 +
            (boot_sector.table_count as u32 * fat_size) + root_dir_sectors as u32;
        let cluster_idx = self.get_data_addr();
        let first_sector = (cluster_idx - 2) * boot_sector.sectors_per_cluster as u32
            + first_data_sector;
        let sector_count = self.file_size / boot_sector.bytes_per_sector as u32 + 1;
        let mut cluster_data = read_sectors(
            mapper, *drive_num, first_sector as u64,
            sector_count as u16
        )?;
        // TODO find better way to return only necessary data
        unsafe { cluster_data.set_len(self.file_size as usize) }
        Ok(cluster_data)
    }
}

pub fn init(drive_num: usize) {
    const BOOT_SECTOR_SIZE: usize = size_of::<FatBootSector>();
    let mapper = crate::MAPPER.get()
        .expect("mapper not initialized");
    let sector_0 = read_sectors(mapper, drive_num, 0, 1)
        .expect("aint no way");
    let boot_sector_bytes = sector_0.into_iter()
        .array_chunks::<BOOT_SECTOR_SIZE>()
        .next()
        .unwrap();

    let boot_sector: FatBootSector = unsafe {
        transmute(boot_sector_bytes)
    };

    let root_cluster = boot_sector.extended_section.root_cluster;
    BOOT_SECTOR.init_once(|| (drive_num, boot_sector));

    let mut fs = Tree::new(File::default());
    parse_directory(mapper,root_cluster, &mut fs.root_mut());

    *FILE_SYSTEM.lock() = Some(FileSystem(fs));
}

fn tree(node: &Node<File>, depth: usize) {
    println!("{}{}", "  ".repeat(depth), node.data().get_name());
    for child in node.iter() {
        tree(child, depth+1);
    }
}

fn parse_directory(
    mapper: &OffsetPageTable, mut cluster_idx: u32, parent: &mut Node<File>
) {
    let (drive_num, boot_sector) = BOOT_SECTOR.get()
        .expect("boot sector not initialized");
    let fat_size = if boot_sector.table_size_16 == 0 {
        boot_sector.extended_section.table_size_32
    } else {
        boot_sector.table_size_16 as u32
    };
    let root_dir_sectors = (boot_sector.root_entry_count*32 + boot_sector.bytes_per_sector-1)
        / boot_sector.bytes_per_sector;
    let first_data_sector = boot_sector.reserved_sector_count as u32 +
        (boot_sector.table_count as u32 * fat_size) + root_dir_sectors as u32;

    loop {
        let first_sector = (cluster_idx - 2) * boot_sector.sectors_per_cluster as u32
            + first_data_sector;
        let cluster_data = read_sectors(
            mapper, *drive_num, first_sector as u64,
            boot_sector.sectors_per_cluster as u16
        ).expect("failed to read root cluster");
        let entries = cluster_data
            .into_iter()
            .array_chunks::<32>();
        for entry in entries {
            if entry[0] == 0 {
                return;
            }
            if entry[0] == 0xE5 {
                continue;
            }
            if entry[11] == 0x0F {
                panic!("detected long filename");
            }

            let file = unsafe {
                *(entry.as_ptr() as *const File)
            };
            // TODO could be error prone detection mechanism
            if file.get_name() == ".          " || file.get_name() == "..         " {
                continue;
            }
            core::mem::forget(entry);
            parent.push_back(Tree::new(file));

            // recursively parse directories
            if file.attributes.directory() == 1 {
                let cluster_addr = file.get_data_addr();
                parse_directory(
                    mapper,cluster_addr, &mut parent.back_mut().unwrap()
                );
            }
        }
        cluster_idx += 1;
    }
}

impl FileSystem {
    pub fn as_tree(&self) -> &Tree<File> {
        &self.0
    }
    fn as_tree_mut(&mut self) -> &mut Tree<File> { &mut self.0 }
}

unsafe impl Send for FileSystem {}