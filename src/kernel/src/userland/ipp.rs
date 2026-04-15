use core::sync::atomic::{AtomicBool, Ordering};

use sink::lockfree::mpsc::Queue;

use crate::{
    interrupt::{CORE_ID, InterruptIndex, LAPIC},
    smp::{CoreId, MAX_CPU},
};

pub struct IppPacketHandler<T> {
    queues: [Queue<T, 256>; MAX_CPU],
    flags: [AtomicBool; MAX_CPU],
}

impl<T> IppPacketHandler<T> {
    pub const fn new() -> Self {
        Self { queues: [const { Queue::new() }; _], flags: [const { AtomicBool::new(false) }; MAX_CPU] }
    }

    fn queues_filtered(&self) -> impl Iterator<Item = (CoreId, &Queue<T, 256>)> {
        let iter = self.queues.iter().enumerate();
        let iter = iter.filter_map(|(core, queue)| CoreId::new(core).map(move |core| (core, queue)));
        iter.filter(|(core, ..)| *core != *CORE_ID)
    }

    fn flags_filtered(&self) -> impl Iterator<Item = (CoreId, &AtomicBool)> {
        let iter = self.flags.iter().enumerate();
        let iter = iter.filter_map(|(core, flags)| CoreId::new(core).map(move |core| (core, flags)));
        iter.filter(|(core, ..)| *core != *CORE_ID)
    }

    pub fn send(&self, mut value: T, core_id: crate::smp::CoreId, urgent: bool) {
        let core = core_id.id();
        self.flags[core].store(false, core::sync::atomic::Ordering::Release);
        while let Err(failed) = self.queues[core].push(value) {
            notify_core(core_id);

            value = failed;
        }

        notify_core(core_id);

        if urgent {
            while !self.flags[core].load(Ordering::Acquire) {
                notify_core(core_id);
            }
        }
    }

    pub fn handle(&self, mut process: impl FnMut(T)) {
        while let Some(c) = self.queues[CORE_ID.id()].pop() {
            process(c)
        }

        self.flags[CORE_ID.id()].store(true, Ordering::Release);
    }
}

impl<T: Clone> IppPacketHandler<T> {
    pub fn broadcast(&self, value: T, urgent: bool) {
        for (_core, flag) in self.flags_filtered() {
            flag.store(false, Ordering::Release);
        }

        for (core, packet) in self.queues_filtered() {
            let mut send = value.clone();
            while let Err(failed) = packet.push(send) {
                notify_core(core);
                send = failed;
            }
        }

        notify_all();

        if urgent {
            while {
                let mut all_handled = true;

                for (core, handled) in self.flags_filtered().map(|(c, f)| (c, f.load(Ordering::Acquire))) {
                    if !handled {
                        notify_core(core);
                    }

                    all_handled &= handled;
                }

                !all_handled
            } {
                core::hint::spin_loop();
            }
        }
    }
}

fn notify_core(core: CoreId) {
    LAPIC.inner_mut().send_fixed_ipi(core, InterruptIndex::CheckIPP);
}

fn notify_all() {
    LAPIC.inner_mut().broadcast_fixed_ipi(InterruptIndex::CheckIPP);
}
