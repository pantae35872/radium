use core::cmp::Reverse;

use alloc::collections::{binary_heap::BinaryHeap, vec_deque::VecDeque};
use derivative::Derivative;

use crate::{
    interrupt::{InterruptIndex, LAPIC, TPMS},
    userland::pipeline::{Event, PipelineContext, TaskBlock},
};

#[derive(Derivative)]
#[derivative(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SleepEntry {
    wakeup_time: usize,
    #[derivative(PartialEq = "ignore", PartialOrd = "ignore", Ord = "ignore")]
    task: TaskBlock,
}

#[derive(Debug, Default)]
pub struct SchedulerPipeline {
    units: VecDeque<TaskBlock>,
    sleep_queue: BinaryHeap<Reverse<SleepEntry>>,

    timer_count: usize,
}

impl SchedulerPipeline {
    pub(super) fn new(events: &mut Event) -> Self {
        events.hw_interrupts(|c, index| {
            if let InterruptIndex::TimerVector = index {
                c.scheduler.handle_timer_interrupt();
            }
        });

        Self::default()
    }

    fn handle_timer_interrupt(&mut self) {
        self.timer_count += 10;
        let tpms = *TPMS;
        LAPIC.inner_mut().reset_timer(tpms * 10);
    }

    pub fn sleep_task(&mut self, task: TaskBlock, amount_millis: usize) {
        let sleep_entry = SleepEntry {
            wakeup_time: self.timer_count + amount_millis,
            task,
        };

        self.sleep_queue.push(Reverse(sleep_entry));
    }

    pub(super) fn add_init(&mut self, init: TaskBlock) {
        self.units.push_back(init);
    }

    pub fn schedule(&mut self, context: &mut PipelineContext) {
        if let Some(task) = context.interrupted_task {
            self.units.push_back(task);
        }
        self.units.extend(&context.added_tasks);

        if self
            .sleep_queue
            .peek()
            .is_some_and(|Reverse(entry)| self.timer_count >= entry.wakeup_time)
        {
            self.units
                .push_front(self.sleep_queue.pop().unwrap().0.task);
        }

        while let Some(task) = self.units.pop_front() {
            if task.valid() {
                context.scheduled_task = Some(task);
                break;
            }
        }
    }
}
