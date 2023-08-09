use alloc::collections::linked_list::{CursorMut, Iter, IterMut};
use alloc::collections::{LinkedList, VecDeque};
use conquer_once::spin::OnceCell;
use lazy_static::lazy_static;
use spin::{Lazy, Mutex, MutexGuard};
use spin::mutex::SpinMutexGuard;
use crate::{hlt_loop, println};
use crate::process::Process;

const QUANTUM: u32 = 20; // timer ticks or about 18.63 ms

pub static SCHEDULER: Mutex<Scheduler> = {
    let tasks = VecDeque::new();
    let scheduler = Scheduler {
        tasks,
        current_task_ticks: 0,
        next_task_id: 0,
        current_task: usize::MAX-1,
        is_enabled: false
    };
    Mutex::new(scheduler)
};

// TODO use Option for current_task rather than invalid value
pub struct Scheduler {
    tasks: VecDeque<Task>,
    current_task_ticks: u32,
    next_task_id: u64,
    current_task: usize,
    is_enabled: bool
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq)]
pub struct PID(u64);

#[derive(Debug, Clone, Copy)]
pub enum TaskState {
    READY,
    RUNNING,
    WAITING,
    DONE,
}

pub struct Task {
    state: TaskState,
    process: Process,
    pid: PID,
}

pub enum TaskKillError {
    DoesNotExist,
}

impl Scheduler {
    pub fn enable(&mut self) {
        self.is_enabled = true;
    }

    pub fn tick(&mut self) {
        if !self.is_enabled {
            return;
        }

        self.current_task_ticks += 1;

        if self.current_task_ticks >= QUANTUM {
            // preempt the process
            let idx = self.current_task;
            if let Some(task) = self.tasks.get_mut(idx) {
                task.state = TaskState::READY;
            }
            unsafe { self.swap_tasks(); }
        }
    }

    /// sets currently executing task to BLOCKED and moves to next one
    pub fn block_current(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.current_task) {
            task.state = TaskState::WAITING;
        }
        unsafe { self.swap_tasks(); }
    }

    unsafe fn swap_tasks(&mut self) {
        if let Some(current_task) = self.tasks.get_mut(self.current_task) {
            let is_resumed = current_task.process.deactivate();
            if is_resumed { return; }
        }
        self.current_task_ticks = 0;

        self.get_next_task();

        if let Some(next_task) = self.tasks.get_mut(self.current_task) {
            next_task.state = TaskState::RUNNING;
            // TODO this is awful
            SCHEDULER.force_unlock();
            next_task.process.activate();
        }
    }

    fn get_next_task(&mut self) -> usize {
        if self.tasks.is_empty() {
            return usize::MAX;
        }

        loop {
            self.current_task += 1;
            if self.current_task >= self.tasks.len() {
                self.current_task = 0;
            }

            if let Some(task) = self.tasks.get(self.current_task) {
                match task.state {
                    TaskState::DONE =>  {
                        self.tasks.remove(self.current_task);
                        self.current_task -= 1;
                    },
                    TaskState::WAITING => continue,
                    TaskState::RUNNING => panic!("Process falsely claims to be running"),
                    TaskState::READY => return self.current_task
                };
            }
        }
    }

    /// adds task to scheduler queue
    ///
    /// returns *unique* process id
    pub fn push_task(&mut self, process: Process) -> PID {
        self.next_task_id += 1;
        let pid = PID(self.next_task_id);
        let task = Task {
            process,
            state: TaskState::READY,
            pid
        };
        self.tasks.push_back(task);
        self.next_task_id += 1;
        pid
    }

    /// removes task with the specified pid
    ///
    /// end_current_task should be preferred as it does not linearly search the task queue
    pub fn end_task(&mut self, pid: PID) -> Result<(), TaskKillError> {
        match self.tasks.iter_mut().find(|task| task.pid == pid) {
            Some(task) => task.state = TaskState::DONE,
            None => return Err(TaskKillError::DoesNotExist),
        }
        Ok(())
    }

    /// removes current task and proceeds to the next task
    ///
    /// cleans the memory used by the process
    pub fn end_current_task(&mut self) -> ! {
        let task = self.tasks.get_mut(self.current_task)
            .expect("failed to get current task");
        task.state = TaskState::DONE;
        unsafe { self.swap_tasks() }

        // in case no tasks are left wait for a timer interrupt to bail out
        hlt_loop();
    }
}
