use crate::userland::pipeline::{PipelineContext, TaskProcesserState, thread::ThreadPipeline};

#[derive(Debug)]
pub struct Dispatcher<'a> {
    state: Option<&'a TaskProcesserState>,
    hlt: bool,
}

#[derive(Debug)]
pub enum DispatchAction<'a> {
    /// Replace the processor state
    ReplaceState(&'a TaskProcesserState),

    /// The dispatch implementor should return to ring zero and hlt
    HltLoop,
}

impl<'a> Dispatcher<'a> {
    pub(super) fn new(context: PipelineContext, thread: &'a ThreadPipeline) -> Self {
        Self { state: context.scheduled_task.map(|e| thread.task_processor_state(e.thread)), hlt: context.should_hlt }
    }

    pub fn dispatch(mut self, mut dispatch: impl FnMut(DispatchAction)) {
        if let Some(state) = self.state.take() {
            dispatch(DispatchAction::ReplaceState(state))
        }

        if self.hlt {
            dispatch(DispatchAction::HltLoop)
        }
    }
}
