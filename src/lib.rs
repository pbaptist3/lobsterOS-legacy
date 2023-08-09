#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(abi_x86_interrupt)]
#![feature(const_mut_refs)]
#![feature(naked_functions)]
#![feature(new_uninit)]
#![feature(arbitrary_self_types)]
#![feature(linked_list_cursors)]
#![feature(error_in_core)]
#![feature(slice_as_chunks)]
#![feature(iter_array_chunks)]
#![feature(vec_into_raw_parts)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
use core::panic::PanicInfo;
use bootloader::BootInfo;
#[cfg(test)]
use bootloader::{entry_point, BootInfo};
use conquer_once::spin::OnceCell;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::VirtAddr;
use crate::threading::scheduler::SCHEDULER;

pub mod display;
pub mod interrupts;
pub mod gdt;
pub mod memory;
pub mod allocator;
pub mod task;
pub mod process;
pub mod threading;
pub mod userspace;
pub mod syscall;
pub mod fs;
pub mod disk;
pub mod acpi;
pub mod pci;
pub mod elf;

#[cfg(test)]
entry_point!(test_kernel_main);

pub static BOOT_INFO: OnceCell<&'static BootInfo> = OnceCell::uninit();
pub static MAPPER: OnceCell<OffsetPageTable> = OnceCell::uninit();

/// initialize the kernel
pub fn init(boot_info: &'static BootInfo) {
    BOOT_INFO.init_once(|| boot_info);

    gdt::init();
    unsafe { syscall::init(); }
    interrupts::init_idt();
    unsafe { interrupts::PICS.lock().initialize() };
    x86_64::instructions::interrupts::enable();

    // create physical mapping and frame allocator
    let mut mapper = unsafe { memory::init() };
    let mut frame_allocator = unsafe {
        memory::BootInfoFrameAllocator::init(&boot_info.memory_map)
    };
    // initialize kernel heap
    allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("Failed to initialize heap");
    MAPPER.init_once(|| mapper);

    acpi::init(boot_info.physical_memory_offset);
    pci::init(boot_info.physical_memory_offset);
    disk::init();
    // TODO make drive num dynamic
    fs::init(1);

    // find shell
    let fs_guard = fs::FILE_SYSTEM.lock();
    let fs = fs_guard.as_ref()
        .expect("file system not initialized");
    let bash_file = fs.as_tree()
        .root()
        .iter()
        .find(|n| {
            let name = n.data().get_name();
            name == "BIN        "
        })
        .expect("no bin folder in root directory")
        .iter()
        .find(|n| {
            let name = n.data().get_name();
            name == "BASH       "
        })
        .expect("no bash executable in /bin")
        .data();
    let data = bash_file.get_data(MAPPER.get().unwrap())
        .expect("failed to get bash data (corrupted disk?)");
    let process = unsafe {
        process::Process::spawn_from_file(&data, &mut frame_allocator)
    };
    let mut scheduler = SCHEDULER.lock();
    scheduler.push_task(process);
    scheduler.enable();
}

/// CPU efficient loop
pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

pub trait Testable {
    fn run(&self);
}

impl<T> Testable for T
    where
        T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[passed]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    hlt_loop();
}

#[cfg(test)]
#[no_mangle]
fn test_kernel_main(boot_info: &'static BootInfo) -> ! {
    test_main();
    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}
