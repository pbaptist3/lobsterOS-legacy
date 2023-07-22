use volatile::Volatile;
use core::fmt;
use core::fmt::Write;
use spin::Mutex;
use lazy_static::lazy_static;
use x86_64::instructions::interrupts;
use crate::serial_println;


const VGA_BUFFER: *mut u16 = 0xb8000 as *mut u16;
const BUFFER_WIDTH: usize = 80;
const BUFFER_HEIGHT: usize = 25;

lazy_static! {
    pub static ref WRITER: Mutex<Writer> = Mutex::new(Writer::new());
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::display::vga_text::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::display::vga_text::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}


#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    // ensure that interrupt does not cause deadlock with mutex
    interrupts::without_interrupts(|| {
        WRITER.lock().write_fmt(args).unwrap();
    });
}

#[derive(Debug, Copy, Clone)]
#[allow(dead_code)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}


#[derive(Clone, Debug, Copy)]
#[repr(transparent)]
pub struct ColorCode(u8);

impl ColorCode {
    pub fn new(foreground: Color, background: Color, blink: bool) -> ColorCode {
        ColorCode((background as u8) << 4 | foreground as u8 | (blink as u8) << 7)
    }
}

/// Represents a character in the vga text buffer
/// Includes both an ascii character and a color code
#[derive(Clone, Debug, Copy)]
#[repr(C)]
#[allow(dead_code)]
pub struct ScreenChar {
    char: u8,
    color: ColorCode,
}

impl ScreenChar {
    pub fn get_char(&self) -> u8 {self.char}
    pub fn get_color(&self) -> ColorCode {self.color}
}

/// VGA text buffer
#[repr(transparent)]
struct Buffer {
    chars: [[Volatile<ScreenChar>; BUFFER_WIDTH]; BUFFER_HEIGHT]
}


pub struct Writer {
    row: usize,
    col: usize,
    default_color: ColorCode,
    buffer: &'static mut Buffer,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            row: 0,
            col: 0,
            default_color: ColorCode::new(Color::White, Color::Black, false),
            buffer: unsafe { &mut *(VGA_BUFFER as *mut Buffer) },
        }
    }

    /// Write a single character to the vga buffer with specified color
    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.new_line();
            }
            _ => {
                let character = ScreenChar {
                    char: byte,
                    color: self.default_color,
                };
                self.buffer.chars[self.row][self.col].write(character);
                self.col+=1;
                // wrap at end of line
                if self.col >= BUFFER_WIDTH {
                    self.new_line();
                }
            }
        }
    }

    /// Write a string to the vga buffer
    pub fn write_string(&mut self, string: &str) {
        for byte in string.bytes() {
            match byte {
                0x20..=0x7e | b'\n' => self.write_byte(byte),
                // invalid ascii characters
                _ => self.write_byte(b'?'),
            }
        }
    }

    /// Swap the currently selected color
    pub fn set_color(&mut self, color: ColorCode) {
        self.default_color = color;
    }

    /// Start a newline and scroll if necessary
    fn new_line(&mut self) {
        self.row += 1;
        self.col = 0;

        // move screen up at end
        if self.row >= BUFFER_HEIGHT {
            self.row = BUFFER_HEIGHT - 1;

            // scroll up
            for row in 1..BUFFER_HEIGHT {
                for col in 0..BUFFER_WIDTH {
                    let character = self.buffer.chars[row][col].read();
                    self.buffer.chars[row - 1][col].write(character);
                }
            }
            self.clear_row(BUFFER_HEIGHT - 1);
        }
    }

    fn clear_row(&mut self, row: usize) {
        let empty = ScreenChar {
            char: b' ',
            color: self.default_color,
        };
        for col in 0..BUFFER_WIDTH {
            self.buffer.chars[row][col].write(empty);
        }
    }

    fn clear_screen(&mut self) {
        for row in 0..BUFFER_HEIGHT {
            self.clear_row(row);
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

#[test_case]
fn test_println_simple() {
    println!("testing\ntesting");
}

#[test_case]
fn test_println_many() {
    for _ in 0..150 {
        println!("testing");
    }
}

#[test_case]
fn test_println_non_ascii() {
    println!("\x00\x12\x7F");
}