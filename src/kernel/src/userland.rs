use self::scheduler::Scheduler;

pub mod scheduler;

pub fn init() {
    scheduler::SCHEDULER.init_once(|| Scheduler::new().into());
}
