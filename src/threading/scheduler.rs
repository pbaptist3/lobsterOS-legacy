use alloc::collections::linked_list::{CursorMut, Iter, IterMut};
use alloc::collections::{LinkedList, VecDeque};
use conquer_once::spin::OnceCell;
use lazy_static::lazy_static;
use spin::{Lazy, Mutex, MutexGuard};
use spin::mutex::SpinMutexGuard;
use crate::println;

use crate::process::{Process, TaskState};
use crate::task::Task;

const QUANTUM: u32 = 20; // timer ticks or about 18.63 ms

pub static SCHEDULER: Mutex<Scheduler> = {
    let tasks = VecDeque::new();
    let scheduler = Scheduler {
        tasks,
        current_task_ticks: 0,
        next_task_id: 0,
        current_task: usize::MAX-1,
    };
    Mutex::new(scheduler)
};

pub struct Scheduler {
    tasks: VecDeque<Process>,
    current_task_ticks: u32,
    next_task_id: u64,
    current_task: usize,
}

impl Scheduler {
    pub fn tick(&mut self) {
        self.current_task_ticks += 1;

        if self.current_task_ticks >= QUANTUM {
            // preempt the process
            let idx = self.current_task;
            if let Some(task) = self.tasks.get_mut(idx) {
                task.update_state(TaskState::READY);
            }
            unsafe { self.swap_tasks(); }
        }
    }

    /// sets currently executing task to BLOCKED and moves to next one
    pub fn block_current(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.current_task) {
            task.update_state(TaskState::WAITING)
        }
        unsafe { self.swap_tasks(); }
    }

    unsafe fn swap_tasks(&mut self) {
        if let Some(current_task) = self.tasks.get_mut(self.current_task) {
            let is_resumed = current_task.deactivate();
            if is_resumed { return; }
        }
        self.current_task_ticks = 0;
        self.get_next_task();
        if let Some(next_task) = self.tasks.get_mut(self.current_task) {
            next_task.update_state(TaskState::RUNNING);
            // TODO this is awful
            SCHEDULER.force_unlock();
            next_task.activate();
        }
    }

    fn get_next_task(&mut self) -> usize {
        use crate::process::TaskState::{DONE, READY, WAITING, RUNNING};

        if self.tasks.is_empty() {
            return usize::MAX;
        }

        loop {
            self.current_task += 1;
            if self.current_task >= self.tasks.len() {
                self.current_task = 0;
            }

            if let Some(task) = self.tasks.get(self.current_task) {
                match task.get_state() {
                    DONE =>  {
                        self.tasks.remove(self.current_task);
                        self.current_task -= 1;
                    },
                    WAITING => continue,
                    RUNNING => panic!("Process falsely claims to be running"),
                    READY => return self.current_task
                };
            }
        }
    }

    /// adds task to scheduler queue and returns process id
    pub fn push_task(&mut self, process: Process) -> PID {
        self.tasks.push_back(process);
        self.next_task_id += 1;
        PID(self.next_task_id)
    }
}

pub struct PID(u64);