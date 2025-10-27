use crate::{
    interrupt::ExtendedInterruptStackFrame,
    userland::pipeline::{ExtendedState, PipelineContext},
};

#[derive(Debug)]
pub struct Dispatcher {}

pub enum DispatchAction {
    ReplaceStackFrame(ExtendedInterruptStackFrame),
    ReplaceExtendedState(ExtendedState),
}

impl Dispatcher {
    pub fn new(context: PipelineContext) -> Self {
        todo!()
    }

    pub fn dispatch(self, _dispatch: impl FnMut(DispatchAction)) {
        todo!()
    }
}
