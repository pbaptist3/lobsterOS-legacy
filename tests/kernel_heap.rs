#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lobster::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use lobster::allocator;
use lobster::memory::{self, BootInfoFrameAllocator};
use x86_64::VirtAddr;
use alloc::boxed::Box;
use alloc::vec::Vec;

entry_point!(main);

fn main(boot_info: &'static BootInfo) -> ! {
    lobster::init();
    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_map) };
    allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap initialization failed");

    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    lobster::test_panic_handler(info)
}

#[test_case]
fn simple_allocation() {
    let heap_1 = Box::new(31);
    let heap_2 = Box::new(12);
    assert_eq!(*heap_1, 31);
    assert_eq!(*heap_2, 12);
}

#[test_case]
fn expanding_vec() {
    let n = 1000;
    let mut vec = Vec::new();
    for i in 0..n {
        vec.push(i);
    }
    assert_eq!(vec.iter().sum::<u64>(), (n - 1) * n / 2);
}

#[test_case]
fn free_memory() {
    for i in 0..10_000 {
        let heap_1 = Box::new(i);
        assert_eq!(*heap_1, i);
    }
}