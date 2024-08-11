use alloc::{string::String, vec::Vec};
use conquer_once::spin::OnceCell;
use spin::Mutex;

pub static SCHEDULER: OnceCell<Mutex<Scheduler>> = OnceCell::uninit();

pub struct Process {
    count: u16,
    prio: usize,
    reset: u16,
    name: String,
}

pub struct Scheduler {
    processes: Vec<Process>,
}

impl Process {
    pub fn new(reset: u16, name: String) -> Self {
        Self {
            count: reset,
            reset,
            prio: 0,
            name,
        }
    }

    pub fn get_name(&self) -> &String {
        &self.name
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            processes: Vec::new(),
        }
    }

    pub fn add_process(&mut self, process: Process) -> &mut Self {
        self.processes.push(process);
        self
    }

    pub fn schedule_next(&mut self) -> Option<&mut Process> {
        if self.processes.is_empty() {
            return None;
        }

        let mut is_zero = false;
        while !is_zero {
            self.processes.iter_mut().for_each(|p| {
                if p.count != 0 {
                    p.count -= 1;
                }
                if p.count == 0 {
                    is_zero = true;
                }
            });
        }

        self.processes.iter_mut().enumerate().for_each(|(_, p)| {
            if p.count == 0 {
                p.prio += 1;
            }
        });
        let (_, process) = self
            .processes
            .iter_mut()
            .enumerate()
            .max_by_key(|(_, process)| process.prio)
            .unwrap();
        process.count = process.reset;
        process.prio = 0;
        return Some(process);
    }
}
