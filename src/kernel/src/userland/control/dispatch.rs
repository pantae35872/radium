use crate::{
    interrupt::ExtendedInterruptStackFrame,
    userland::control::{ExtendedState, TaskBlock},
};

#[derive(Debug)]
pub struct Dispatcher {}

pub enum DispatchAction {
    ReplaceStackFrame(ExtendedInterruptStackFrame),
    ReplaceExtendedState(ExtendedState),
}

impl Dispatcher {
    pub fn new(_task_block: TaskBlock) -> Self {
        todo!()
    }

    pub fn dispatch(self, _dispatch: impl FnMut(DispatchAction)) {
        todo!()
    }
}
