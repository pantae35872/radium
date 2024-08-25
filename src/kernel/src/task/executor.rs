use core::task::{Context, Poll, Waker};

use alloc::{sync::Arc, task::Wake};
use crossbeam_queue::ArrayQueue;
use hashbrown::HashMap;
use x86_64::instructions::interrupts;

use super::{AwaitType, Task, TaskId};

pub struct Executor {
    tasks: HashMap<TaskId, Task>,
    task_queue: Arc<ArrayQueue<TaskId>>,
    waker_cache: HashMap<TaskId, Waker>,
}

impl Executor {
    pub fn new() -> Self {
        Executor {
            tasks: HashMap::new(),
            task_queue: Arc::new(ArrayQueue::new(100)),
            waker_cache: HashMap::new(),
        }
    }

    pub fn run(&mut self) -> ! {
        loop {
            self.run_ready_tasks();
            self.sleep_if_idle();
        }
    }

    pub fn run_exit(&mut self) {
        loop {
            self.run_ready_tasks();
            if self.tasks.is_empty() {
                break;
            }
        }
    }

    pub fn spawn(&mut self, task: Task) {
        let task_id = task.id;
        if self.tasks.insert(task.id, task).is_some() {
            panic!("Task with same ID already in tasks");
        }
        self.task_queue.push(task_id).expect("queue full");
    }

    fn run_ready_tasks(&mut self) {
        let Self {
            tasks,
            task_queue,
            waker_cache,
        } = self;
        while let Ok(task_id) = task_queue.pop() {
            let task = match tasks.get_mut(&task_id) {
                Some(task) => task,
                None => continue,
            };
            let waker = waker_cache
                .entry(task_id)
                .or_insert_with(|| TaskWaker::new(task_id, task_queue.clone()));
            let mut context = Context::from_waker(waker);
            match task.poll(&mut context) {
                Poll::Ready(()) => {
                    tasks.remove(&task_id);
                    waker_cache.remove(&task_id);
                }
                Poll::Pending => match task.typ {
                    AwaitType::Poll => task_queue.push(task_id).expect("Queue is full"),
                    _ => {}
                },
            }
        }
    }

    fn sleep_if_idle(&self) {
        interrupts::disable();
        if self.task_queue.is_empty() {
            interrupts::enable_and_hlt();
        } else {
            interrupts::enable();
        }
    }
}

struct TaskWaker {
    task_id: TaskId,
    task_queue: Arc<ArrayQueue<TaskId>>,
}

impl TaskWaker {
    fn new(task_id: TaskId, task_queue: Arc<ArrayQueue<TaskId>>) -> Waker {
        Waker::from(Arc::new(TaskWaker {
            task_id,
            task_queue,
        }))
    }

    fn wake_task(&self) {
        self.task_queue.push(self.task_id).expect("task_queue full");
    }
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_task();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wake_task();
    }
}
