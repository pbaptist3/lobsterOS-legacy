#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lobster::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use lobster::{println, process, serial_print, serial_println};
use core::panic::PanicInfo;
use lobster::hlt_loop;
use bootloader::{BootInfo, entry_point};
use x86_64::VirtAddr;
use lobster::memory;
use lobster::allocator;
use lobster::process::Process;
use lobster::threading::scheduler::{Scheduler, SCHEDULER};

entry_point!(kernel_main);

/// Entry point of kernel; should call initializer
fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("Starting LobsterOS");
    // general initialization
    lobster::init(boot_info);

    #[cfg(test)]
    test_main();

    /*let mut executor = Executor::new();
    executor.spawn(Task::new(keyboard::print_keypresses()));
    executor.run();*/

    /*unsafe {
        let example_process = Process::new(&mut mapper, &mut frame_allocator);
        let ex_process_2 = Process::new(&mut mapper, &mut frame_allocator);
        let _pid = SCHEDULER.lock().push_task(example_process);
        let _pid2 = SCHEDULER.lock().push_task(ex_process_2);
    };*/

    hlt_loop()
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("Panicked! {:?}", info);
    println!("Panicked! {:?}", info);

    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    lobster::test_panic_handler(info);
}