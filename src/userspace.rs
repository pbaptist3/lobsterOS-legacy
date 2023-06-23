use core::arch::asm;
use crate::{println, serial_println};

pub unsafe fn example_process() {
    loop {}
    asm!("\
        mov rax, 0x02
        mov rdi, 0x04
        mov rsi, 0x08
        mov rdx, 0x10
        mov r10, 0x20
        syscall
    ");
    let a = 5;
    let b = 3;
    let c = a * b * b;
    asm!("\
        mov rax, 0x03
        mov rdi, 0x06
        mov rsi, 0x0C
        mov rdx, 0x18
        mov r10, rcx
        syscall
    ", in("rcx") c);
    loop {
        x86_64::instructions::hlt();
    }
}