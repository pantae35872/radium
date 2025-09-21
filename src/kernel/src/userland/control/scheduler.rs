use crate::userland::control::TaskProcessor;

#[repr(C)]
#[derive(Debug)]
pub struct Thread {
    global_id: usize,
}

#[derive(Debug)]
pub struct SchedulerProcessor {}

impl SchedulerProcessor {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for SchedulerProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskProcessor for SchedulerProcessor {
    fn update_task(&mut self, task: &mut super::TaskBlock) {}

    fn finalize_update(&mut self, task: &super::TaskBlock) {}
}
