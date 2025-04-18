use alloc::vec::Vec;

struct CpuQueue {
    hi_prio: Vec<Thread>,
    mid_prio: Vec<Thread>,
    low_prio: Vec<Thread>,
}

struct MainScheduler {
    queues: Vec<CpuQueue>,
}

struct Thread {}

fn init() {}
