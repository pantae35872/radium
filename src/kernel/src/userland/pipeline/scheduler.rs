use core::{
    cmp::Reverse,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::collections::{binary_heap::BinaryHeap, vec_deque::VecDeque};
use derivative::Derivative;

use crate::{
    interrupt::{InterruptIndex, CORE_ID, LAPIC, TPMS},
    smp::{CoreId, MAX_CPU},
    userland::pipeline::{thread::ThreadPipeline, Event, PipelineContext, TaskBlock},
};

const MIGRATION_THRESHOLD: usize = 2;

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
    scheduled_count: usize,

    timer_count: usize,
}

impl SchedulerPipeline {
    pub(super) fn new(events: &mut Event) -> Self {
        events.hw_interrupts(|c, index| {
            if let InterruptIndex::TimerVector = index {
                c.scheduler.handle_timer_interrupt();
            }
        });

        events.finalize(|c, cx| {
            c.scheduler.finalize(cx);
        });

        events.begin(|c, pc, _rqc| {
            if let Some(task) = pc.interrupted_task {
                c.scheduler.units.push_back(task);
            }
        });

        Self::default()
    }

    fn finalize(&mut self, context: &mut PipelineContext) {
        self.units.extend(&context.added_tasks);
    }

    pub fn timer_count(&self) -> usize {
        self.timer_count
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

    pub(super) fn add_task(&mut self, init: TaskBlock) {
        self.units.push_back(init);
    }

    fn migrate(&mut self, thread: &mut ThreadPipeline) {
        let local_core = CORE_ID.id();
        let local_count = self.units.len();

        TASK_COUNT_EACH_CORE[local_core].store(local_count, Ordering::Relaxed);

        let mut target_core = usize::MAX;
        let mut min_count = usize::MAX;

        for (core_id, count) in TASK_COUNT_EACH_CORE.iter().enumerate() {
            let count = count.load(Ordering::Relaxed);

            if core_id == local_core || count == usize::MAX {
                continue;
            }

            if count < min_count {
                min_count = count;
                target_core = core_id;
            }
        }

        if target_core == usize::MAX || local_count <= min_count + MIGRATION_THRESHOLD {
            return;
        }

        while let Some(task) = self.units.pop_front() {
            if !task.valid() {
                continue;
            }

            let core = CoreId::new(target_core)
                .expect("Unintialized core selected when calcuating thread migration");

            thread.migrate(core, task);

            TASK_COUNT_EACH_CORE[local_core].fetch_sub(1, Ordering::Relaxed);
            TASK_COUNT_EACH_CORE[target_core].fetch_add(1, Ordering::Relaxed);
            break;
        }
    }

    pub fn schedule(&mut self, thread: &mut ThreadPipeline, context: &mut PipelineContext) {
        self.migrate(thread);

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

static TASK_COUNT_EACH_CORE: [AtomicUsize; MAX_CPU] =
    [const { AtomicUsize::new(usize::MAX) }; MAX_CPU];
