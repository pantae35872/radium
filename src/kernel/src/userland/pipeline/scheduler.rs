use core::{
    cmp::Reverse,
    sync::atomic::{AtomicUsize, Ordering}, 
};

use alloc::collections::{binary_heap::BinaryHeap, vec_deque::VecDeque};
use derivative::Derivative;

use crate::{
    interrupt::{CORE_ID, InterruptIndex, LAPIC, TPMS}, serial_print, serial_println, smp::{CoreId, MAX_CPU}, userland::pipeline::{Event, PipelineContext, TaskBlock, thread::ThreadPipeline}
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

        Self::default()
    }

    fn finalize(&mut self, context: &mut PipelineContext) {
        self.units.extend(&context.added_tasks);

        context.should_hlt = (context.should_schedule || context.interrupted_slept) && context.scheduled_task.is_none();
    }

    pub fn timer_count(&self) -> usize {
        self.timer_count
    }

    fn handle_timer_interrupt(&mut self) {
        self.timer_count += 10;
        let tpms = *TPMS;
        LAPIC.inner_mut().reset_timer(tpms * 10);
    }

    pub fn sleep_interrupted(&mut self, context: &mut PipelineContext, amount_millis: usize) {
        assert!(context.interrupted_task.is_some(), "sleep interrupted called with no interrupted task");
        let sleep_entry = SleepEntry {
            wakeup_time: self.timer_count + amount_millis,
            task: context.interrupted_task.unwrap(),
        };

        self.sleep_queue.push(Reverse(sleep_entry));
        context.interrupted_slept = true;
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

        while let Some(task) = self.units.pop_back() {
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
        if let Some(interrupted_task) = context.interrupted_task && !context.interrupted_slept {
            self.units.push_back(interrupted_task);
        }

        self.migrate(thread);

        if self
            .sleep_queue
            .peek()
            .is_some_and(|Reverse(entry)| self.timer_count >= entry.wakeup_time)
        {
            let task = self.sleep_queue.pop().unwrap().0.task;
            self.units.push_front(task);
        }

        if CORE_ID.is_bsp() {
            TASK_COUNT_EACH_CORE.iter().filter_map(|t| match t.load(Ordering::Relaxed) {
                usize::MAX => None,
                t => Some(t),
            }).enumerate().for_each(|(core, task)| serial_print!("[Core {core} has {task} tasks] "));
            serial_println!();
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
