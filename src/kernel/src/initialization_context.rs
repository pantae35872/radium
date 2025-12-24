use alloc::vec::Vec;
use bootbridge::BootBridge;
use pager::paging::{ActivePageTable, table::RecurseLevel4, temporary_page::TemporaryPage};

use crate::{
    driver::acpi::{Acpi, madt::IoApicInterruptSourceOverride},
    interrupt::{apic::ApicId, io_apic::IoApicManager},
    memory::{
        MMIOBufferInfo, allocator::buddy_allocator::BuddyAllocator, stack_allocator::StackAllocator,
    },
    port::PortAllocator,
    smp::LocalInitializer,
};

macro_rules! create_initialization_chain {
    (
        $first_phase: ident {
            $($first_field:ident : $first_ty: ty),* $(,)?
        }
        $(=> $next_phase:ident {
            $($next_field:ident : $next_ty: ty),* $(,)?
        })*
    ) => {
        #[allow(unused_parens)]
        impl AnyInitializationPhase for $first_phase {
            type PreviousPhase = ();
            type Additional = ($($first_ty),*);

            fn create(_previous: Self::PreviousPhase, additional: Self::Additional) -> Self {
                let ($($first_field),*) = additional;
                Self {
                    $($first_field),*
                }
            }
        }

        create_initialization_chain!(@accum
            [$($first_field : $first_ty),*]
            [$first_phase {
                $($first_field : $first_ty),*
            }]
            $(=> $next_phase {
                $($next_field : $next_ty),*
            })*
        );
    };

    (@accum
        [$($accum_field:ident : $accum_ty:ty),* $(,)?]
        [$previous_phase: ident { $($prev_field:ident : $prev_ty:ty),* $(,)? }]
        => $current_phase: ident {
           $($current_field:ident : $current_ty: ty),* $(,)?
        }
        $(=> $rest_phase:ident {
           $($rest_field:ident : $rest_ty:ty),* $(,)?
        })*
    ) => {
        pub struct $previous_phase {
            $(pub $accum_field : $accum_ty),*
        }

        impl $previous_phase {
            $(
                pub fn $accum_field(&self) -> &$accum_ty {
                    &self.$accum_field
                }
            )*
        }

        #[allow(unused_parens)]
        impl AnyInitializationPhase for $current_phase {
            type PreviousPhase = $previous_phase;
            type Additional = ($($current_ty),*);

            fn create(previous: Self::PreviousPhase, additional: Self::Additional) -> Self {
                let ($($current_field),*) = additional;
                Self {
                    $($prev_field: previous.$prev_field.into(),)*
                    $($current_field: $current_field.into()),*
                }
            }
        }

        #[allow(unused_parens)]
        impl InitializationPhase for $previous_phase {
            type Next = $current_phase;
            type Additional = ($($current_ty),*);

            fn next(self, additional: Self::Additional) -> Self::Next {
                <$current_phase as AnyInitializationPhase>::create(self, additional)
            }
        }

        create_initialization_chain!(@accum
            [$($accum_field : $accum_ty,)* $($current_field : $current_ty,)*]
            [$current_phase {
                $($accum_field : $accum_ty,)*
                $($current_field : $current_ty,)*
            }]
            $(=> $rest_phase {
                $($rest_field : $rest_ty,)*
            })*
        );
    };

    (@accum
        [$($accum_field:ident : $accum_ty:ty),* $(,)?]
        [$last_phase:ident {
            $($last_field:ident : $last_ty:ty),* $(,)?
        }]
    ) => {
        pub struct $last_phase {
            $(pub $accum_field : Option<$accum_ty>,)*
        }

        impl $last_phase {
            $(
                paste::paste! {
                    pub fn [< take_$accum_field >](&mut self) -> Option<$accum_ty> {
                        self.$accum_field.take()
                    }
                }

                pub fn $accum_field(&self) -> Option<&$accum_ty> {
                    self.$accum_field.as_ref()
                }
            )*
        }
   }
}

macro_rules! select_context {
    ($(
        ($($phase:ident),*) => $body:tt
    )*) => {$(
        $(
            impl $crate::initialization_context::InitializationContext<$crate::initialization_context::$phase> $body
        )*
    )*};
}

#[allow(unused_imports)]
pub(crate) use select_context;

create_initialization_chain! {
    Stage0 {
        boot_bridge: BootBridge,
        port_allocator: PortAllocator,
    } => Stage1 {
        active_table: ActivePageTable<RecurseLevel4>,
        buddy_allocator: BuddyAllocator<64>,
        stack_allocator: StackAllocator,
        temporary_page: TemporaryPage,
    } => Stage2 {
        processors: Vec<ApicId>,
        local_apic_mmio: MMIOBufferInfo,
        io_apics: Vec<(MMIOBufferInfo, usize)>,
        interrupt_source_overrides: Vec<IoApicInterruptSourceOverride>,
        acpi: Acpi,
    } => Stage3 {
        local_initializer: Option<LocalInitializer>,
    } => Stage4 {
        io_apic_manager: IoApicManager,
    } => End {}
}

pub trait AnyInitializationPhase {
    type PreviousPhase;
    type Additional;

    fn create(previous: Self::PreviousPhase, additional: Self::Additional) -> Self;
}

pub trait InitializationPhase {
    type Additional;
    type Next: AnyInitializationPhase<Additional = Self::Additional, PreviousPhase = Self>;

    fn next(self, additional: Self::Additional) -> Self::Next
    where
        Self: Sized,
    {
        Self::Next::create(self, additional)
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct InitializationContext<T: AnyInitializationPhase> {
    pub context: T,
}

impl<T: AnyInitializationPhase> InitializationContext<T> {
    pub fn start(boot_bridge: BootBridge) -> InitializationContext<Stage0> {
        InitializationContext {
            context: Stage0 {
                boot_bridge,
                port_allocator: PortAllocator::new(),
            },
        }
    }

    pub fn context(&self) -> &T {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut T {
        &mut self.context
    }

    pub fn next(
        self,
        next: <T as InitializationPhase>::Additional,
    ) -> InitializationContext<T::Next>
    where
        T: InitializationPhase,
    {
        InitializationContext {
            context: self.context.next(next),
        }
    }
}
