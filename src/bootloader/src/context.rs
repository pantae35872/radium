use bakery::DwarfBaker;

macro_rules! create_initialization_chain {
    (
        $first_stage: ident {
            $($first_field:ident : $first_ty: ty),* $(,)?
        }
        $(=> $next_stage:ident {
            $($next_field:ident : $next_ty: ty),* $(,)?
        })*
    ) => {
        #[allow(unused_parens)]
        impl AnyInitializationStage for $first_stage {
            type PreviousStage = ();
            type Additional = ($($first_ty),*);

            fn create(_previous: Self::PreviousStage, additional: Self::Additional) -> Self {
                let ($($first_field),*) = additional;
                Self {
                    $($first_field),*
                }
            }
        }

        create_initialization_chain!(@accum
            [$($first_field : $first_ty),*]
            [$first_stage {
                $($first_field : $first_ty),*
            }]
            $(=> $next_stage {
                $($next_field : $next_ty),*
            })*
        );
    };

    (@accum
        [$($accum_field:ident : $accum_ty:ty),*]
        [$previous_stage: ident { $($prev_field:ident : $prev_ty:ty),* }]
        => $current_stage: ident {
           $($current_field:ident : $current_ty: ty),*
        }
        $(=> $rest_stage:ident {
           $($rest_field:ident : $rest_ty:ty),*
        })*
    ) => {
        pub struct $previous_stage {
            $(pub $accum_field : $accum_ty),*
        }

        impl $previous_stage {
            $(
                pub fn $accum_field(&self) -> &$accum_ty {
                    &self.$accum_field
                }
            )*
        }

        #[allow(unused_parens)]
        impl AnyInitializationStage for $current_stage {
            type PreviousStage = $previous_stage;
            type Additional = ($($current_ty),*);

            fn create(previous: Self::PreviousStage, additional: Self::Additional) -> Self {
                let ($($current_field),*) = additional;
                Self {
                    $($prev_field: previous.$prev_field,)*
                    $($current_field),*
                }
            }
        }

        #[allow(unused_parens)]
        impl InitializationStage for $previous_stage {
            type Next = $current_stage;
            type Additional = ($($current_ty),*);

            fn next(self, additional: Self::Additional) -> Self::Next {
                <$current_stage as AnyInitializationStage>::create(self, additional)
            }
        }

        create_initialization_chain!(@accum
            [$($accum_field : $accum_ty,)* $($current_field : $current_ty),*]
            [$current_stage {
                $($accum_field : $accum_ty,)*
                $($current_field : $current_ty),*
            }]
            $(=> $rest_stage {
                $($rest_field : $rest_ty),*
            })*
        );
    };

    (@accum
        [$($accum_field:ident : $accum_ty:ty),*]
        [$last_stage:ident { $($last_field:ident : $last_ty:ty),* }]
    ) => {
        pub struct $last_stage {
            $(pub $accum_field : $accum_ty),*
        }

        impl $last_stage {
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
        ($($stage:ident),*) => $body:tt
    )*) => {$(
        $(
            impl $crate::context::InitializationContext<$crate::context::$stage> $body
        )*
    )*};
}

use bootbridge::{GraphicsInfo, RawData};
use packery::Packed;
use pager::{
    address::{PhysAddr, VirtAddr},
    allocator::linear_allocator::LinearAllocator,
    paging::{
        mapper::Mapper,
        table::{RootDirect, Table},
    },
};
use santa::Elf;
#[allow(unused_imports)]
pub(crate) use select_context;

create_initialization_chain! {
    Stage1 {
        font_data: RawData,
        dwarf_data: DwarfBaker<'static>,
        packed: Packed<'static>,
        rsdp: PhysAddr,
        kernel_file: &'static [u8],
    } => Stage2 {
        elf: Elf<'static>,
    } => Stage3 {
        table: *mut Table<RootDirect>,
        temporary_runtime_allocator: LinearAllocator,
        entry: VirtAddr,
    } => Stage4 {
        frame_buffer: RawData,
        graphics_info: GraphicsInfo,
    } => Stage5 {
        entry_size: usize,
        runtime_service: u64,
    }
}

pub trait AnyInitializationStage {
    type PreviousStage;
    type Additional;

    fn create(previous: Self::PreviousStage, additional: Self::Additional) -> Self;
}

pub trait InitializationStage {
    type Additional;
    type Next: AnyInitializationStage<Additional = Self::Additional, PreviousStage = Self>;

    fn next(self, additional: Self::Additional) -> Self::Next
    where
        Self: Sized,
    {
        Self::Next::create(self, additional)
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct InitializationContext<T: AnyInitializationStage> {
    pub context: T,
}

select_context!(
    (Stage3, Stage4, Stage5) => {
        pub fn mapper(&self) -> Mapper<RootDirect> {
            unsafe { Mapper::new_custom(self.context().table) }
        }
    }
);

impl<T: AnyInitializationStage> InitializationContext<T> {
    pub fn start(
        font_data: RawData,
        dwarf_data: DwarfBaker<'static>,
        packed: Packed<'static>,
        rsdp: PhysAddr,
        kernel_file: &'static [u8],
    ) -> InitializationContext<Stage1> {
        InitializationContext { context: Stage1 { font_data, dwarf_data, packed, rsdp, kernel_file } }
    }

    pub fn context(&self) -> &T {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut T {
        &mut self.context
    }

    pub fn next(self, next: <T as InitializationStage>::Additional) -> InitializationContext<T::Next>
    where
        T: InitializationStage,
    {
        InitializationContext { context: self.context.next(next) }
    }
}
