use alloc::boxed::Box;

const STACK_SIZE: usize = 1024 * 16;

pub struct Thread {
    code: Box<dyn Fn() -> ()>,
    stack: Box<[u8]>,
    state: State,
}

impl Thread {
    /// Create a new thread that has the given code and stack
    /// Thread begins in a ready state
    pub fn new<T>(code: T) -> Self
    where
        T: Fn() -> () + 'static,
    {
        Self {
            code: Box::new(code),
            stack: Box::new([0u8; STACK_SIZE]),
            state: State::READY,
        }
    }

    pub fn run(&mut self) {

    }
}

pub enum State {
    READY,
}
