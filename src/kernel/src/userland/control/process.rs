use crate::userland::control::TaskProcessor;

#[derive(Debug)]
pub struct Process {}

#[derive(Debug)]
pub struct ProcessProcessor {}

impl ProcessProcessor {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for ProcessProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskProcessor for ProcessProcessor {
    fn update_task(&mut self, task: &mut super::TaskBlock) {
        todo!()
    }

    fn finalize_update(&mut self, task: &super::TaskBlock) {
        todo!()
    }
}
