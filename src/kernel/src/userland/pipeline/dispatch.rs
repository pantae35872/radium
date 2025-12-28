use crate::userland::pipeline::{PipelineContext, TaskProcesserState, thread::ThreadPipeline};

#[derive(Debug)]
pub struct Dispatcher<'a> {
    state: Option<&'a TaskProcesserState>,
}

pub enum DispatchAction<'a> {
    ReplaceState(&'a TaskProcesserState),
}

impl<'a> Dispatcher<'a> {
    pub fn new(context: PipelineContext, thread: &'a ThreadPipeline) -> Self {
        Self {
            state: context
                .scheduled_task
                .map(|e| thread.task_processor_state(e.thread)),
        }
    }

    pub fn dispatch(mut self, mut dispatch: impl FnMut(DispatchAction)) {
        if let Some(state) = self.state.take() {
            dispatch(DispatchAction::ReplaceState(state))
        }
    }
}
