#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lobster::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use lobster::{println, process, serial_println};
use core::panic::PanicInfo;
use lobster::hlt_loop;
use bootloader::{BootInfo, entry_point};
use x86_64::VirtAddr;
use lobster::memory;
use lobster::allocator;
use lobster::task::executor::Executor;
use lobster::task::{keyboard, Task};

entry_point!(kernel_main);


/// Entry point of kernel; should call initializer
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Starting LobsterOS");
    // general initialization
    lobster::init();

    // create physical mapping and frame allocator
    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe {
        memory::BootInfoFrameAllocator::init(&boot_info.memory_map)
    };
    // initialize kernel heap
    allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("Failed to initialize heap");

    #[cfg(test)]
    test_main();

    /*let mut executor = Executor::new();
    executor.spawn(Task::new(keyboard::print_keypresses()));
    executor.run();*/

    unsafe {
        process::Process::new(&mut mapper, &mut frame_allocator)
    };

    loop {}
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("Panicked! {:?}", info);

    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    lobster::test_panic_handler(info);
}