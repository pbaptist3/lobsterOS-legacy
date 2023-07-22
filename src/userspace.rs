use alloc::ffi::CString;
use core::arch::asm;

pub unsafe fn example_process() {
    loop {}
    let print = |text: &[u8]| {
        asm!(
            "mov rax, 0x0
            mov rdi, {0}
            mov rsi, {1}
            syscall",
            in(reg) text.as_ptr() as u64,
            in(reg) text.len()
        );
    };
    let mut count = 0;
    loop {
        count += 1;
        print(b"test");
    }
    //loop {}
    /*asm!("\
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
        nop
    ", in("rcx") c);
    loop {
        x86_64::instructions::hlt();
    }*/
}