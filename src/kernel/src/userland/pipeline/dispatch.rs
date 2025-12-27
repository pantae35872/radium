use crate::{
    interrupt::ExtendedInterruptStackFrame,
    userland::pipeline::{ExtendedState, PipelineContext, thread::ThreadPipeline},
};

#[derive(Debug)]
pub struct Dispatcher {
    stack_frame: Option<ExtendedInterruptStackFrame>,
    extended_state: Option<ExtendedState>,
}

pub enum DispatchAction {
    ReplaceStackFrame(ExtendedInterruptStackFrame),
    ReplaceExtendedState(ExtendedState),
}

impl Dispatcher {
    pub fn new(_context: PipelineContext, _thread: &ThreadPipeline) -> Self {
        todo!()
        //Self {
        //    stack_frame: context
        //        .scheduled_task
        //        .map(|e| thread.task_processor_state(e.thread))
        //    extended_state: (),
        //}
    }

    pub fn dispatch(mut self, mut dispatch: impl FnMut(DispatchAction)) {
        if let Some(stack_frame) = self.stack_frame.take() {
            dispatch(DispatchAction::ReplaceStackFrame(stack_frame))
        }

        if let Some(state) = self.extended_state.take() {
            dispatch(DispatchAction::ReplaceExtendedState(state))
        }
    }
}
