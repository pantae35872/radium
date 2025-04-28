use alloc::vec::Vec;
use bootbridge::BootBridge;
use pager::paging::{table::RecurseLevel4, ActivePageTable};

use crate::{
    driver::acpi::{madt::IoApicInterruptSourceOverride, Acpi},
    memory::{
        allocator::buddy_allocator::BuddyAllocator, stack_allocator::StackAllocator, MMIOBufferInfo,
    },
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
        [$($accum_field:ident : $accum_ty:ty),*]
        [$previous_phase: ident { $($prev_field:ident : $prev_ty:ty),* }]
        => $current_phase: ident {
           $($current_field:ident : $current_ty: ty),*
        }
        $(=> $rest_phase:ident {
           $($rest_field:ident : $rest_ty:ty),*
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
                    $($prev_field: previous.$prev_field,)*
                    $($current_field),*
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
            [$($accum_field : $accum_ty,)* $($current_field : $current_ty),*]
            [$current_phase {
                $($accum_field : $accum_ty,)*
                $($current_field : $current_ty),*
            }]
            $(=> $rest_phase {
                $($rest_field : $rest_ty),*
            })*
        );
    };

    (@accum
        [$($accum_field:ident : $accum_ty:ty),*]
        [$last_phase:ident { $($last_field:ident : $last_ty:ty),* }]
    ) => {
        pub struct $last_phase {
            $(pub $accum_field : $accum_ty),*
        }

        impl $last_phase {
            $(
                pub fn $accum_field(&self) -> &$accum_ty {
                    &self.$accum_field
                }
            )*
        }
   }
}

macro_rules! select_context {
    ($(
        ($($phase:ty),*) => $body:tt
    )*) => {$(
        $(
            impl InitializationContext<$phase> $body
        )*
    )*};
}

#[allow(unused_imports)]
pub(crate) use select_context;

create_initialization_chain! {
    Phase0 {
        boot_bridge: BootBridge,
    } => Phase1 {
        active_table: ActivePageTable<RecurseLevel4>,
        buddy_allocator: BuddyAllocator<64>,
        stack_allocator: StackAllocator,
    } => Phase2 {
        processors: Vec<usize>,
        local_apic_mmio: MMIOBufferInfo,
        io_apics: Vec<(MMIOBufferInfo, usize)>,
        interrupt_source_overrides: Vec<IoApicInterruptSourceOverride>,
        acpi: Acpi,
    } => Phase3 {
        local_initializer: Option<LocalInitializer>,
    }
}

impl AsMut<Phase1> for Phase1 {
    fn as_mut(&mut self) -> &mut Phase1 {
        // SAFETY: We know this is safe from the macro
        unsafe { core::mem::transmute(self) }
    }
}

impl AsMut<Phase1> for Phase3 {
    fn as_mut(&mut self) -> &mut Phase1 {
        // SAFETY: We know this is safe from the macro
        unsafe { core::mem::transmute(self) }
    }
}

impl AsRef<Phase1> for Phase3 {
    fn as_ref(&self) -> &Phase1 {
        // SAFETY: We know this is safe from the macro
        unsafe { core::mem::transmute(self) }
    }
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

impl<B, T> AsMut<InitializationContext<B>> for InitializationContext<T>
where
    T: AsMut<B> + AnyInitializationPhase,
    B: AnyInitializationPhase,
{
    fn as_mut(&mut self) -> &mut InitializationContext<B> {
        // SAFETY: We know this is safe from the macro
        unsafe { core::mem::transmute(self) }
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct InitializationContext<T: AnyInitializationPhase> {
    pub context: T,
}

impl<T: AnyInitializationPhase> InitializationContext<T> {
    pub fn start(boot_bridge: BootBridge) -> InitializationContext<Phase0> {
        InitializationContext {
            context: Phase0 { boot_bridge },
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
