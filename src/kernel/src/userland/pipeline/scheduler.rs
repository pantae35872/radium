use alloc::vec::Vec;

use crate::userland::pipeline::{PipelineContext, TaskBlock, thread::ThreadPipeline};

#[derive(Debug)]
struct SchedulerUnit {
    block: TaskBlock,
}

#[derive(Debug)]
pub struct SchedulerPipeline {
    units: Vec<SchedulerUnit>,
}

impl SchedulerPipeline {
    pub fn new() -> Self {
        Self { units: Vec::new() }
    }

    pub fn schedule(&mut self, _context: &mut PipelineContext, _thread: &mut ThreadPipeline) {}
}

impl Default for SchedulerPipeline {
    fn default() -> Self {
        Self::new()
    }
}
