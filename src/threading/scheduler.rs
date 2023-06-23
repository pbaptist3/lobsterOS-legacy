use alloc::collections::LinkedList;
use alloc::vec::Vec;
use crate::process::Process;

struct Scheduler {
    tasks: LinkedList<Process>
}
