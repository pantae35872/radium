use core::mem::forget;

use bakery::DwarfBaker;
use boot_cfg_parser::toml::parser::TomlValue;

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

use bootbridge::{BootBridgeBuilder, GraphicsInfo, KernelConfig, MemoryMap, RawBootBridge, RawData};
use packery::Packed;
use pager::{
    address::PhysAddr,
    allocator::linear_allocator::LinearAllocator,
    paging::{
        ActivePageTable,
        table::{DirectLevel4, Table},
    },
};
use santa::Elf;
#[allow(unused_imports)]
pub(crate) use select_context;

use crate::config::BootConfig;

create_initialization_chain! {
    Stage0 {
        config: TomlValue,
    } => Stage1 {
        font_data: RawData,
        dwarf_data: DwarfBaker<'static>,
        packed: Packed<'static>,
        rsdp: PhysAddr,
        kernel_config: KernelConfig,
        kernel_file: &'static [u8],
    } => Stage2 {
        entry_point: u64,
        kernel_base: PhysAddr,
        elf: Elf<'static>,
    } => Stage3 {
        table: u64,
        allocator: LinearAllocator,
    } => Stage4 {
        frame_buffer: RawData,
        graphics_info: GraphicsInfo,
    } => Stage5 {
        entry_size: usize,
        runtime_service: u64,
    } => Stage6 {
        memory_map: MemoryMap<'static>,
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
    (Stage0, Stage1, Stage2, Stage3, Stage4, Stage5, Stage6) => {
        pub fn config(&self) -> BootConfig<'_> {
            BootConfig::parse(self.context().config())
        }
    }
    (Stage3, Stage4, Stage5, Stage6) => {
        pub fn active_table(&self) -> ActivePageTable<DirectLevel4> {
            unsafe { ActivePageTable::new_custom(self.context().table as *mut Table<DirectLevel4>) }
        }
    }
);

impl InitializationContext<Stage6> {
    pub fn build_bridge(self, mut builder: BootBridgeBuilder) -> *mut RawBootBridge {
        let mut table = self.active_table();
        // NOTE: The config must be forget or otherwise it'll call deallocate which will crash
        // after exiting boot service
        forget(self.context.config);

        let Stage6 {
            elf,
            dwarf_data,
            mut allocator,
            packed,
            memory_map,
            font_data,
            graphics_info,
            kernel_config,
            kernel_base,
            frame_buffer,
            runtime_service,
            rsdp,
            ..
        } = self.context;
        table.identity_map_object(&elf, &mut allocator);
        table.identity_map_object(&dwarf_data, &mut allocator);
        table.identity_map_object(&builder, &mut allocator);

        builder
            .memory_map(memory_map)
            .font_data(font_data)
            .early_alloc(allocator)
            .graphics_info(graphics_info)
            .kernel_config(kernel_config)
            .kernel_base(kernel_base)
            .framebuffer_data(frame_buffer)
            .runtime_service(runtime_service)
            .rsdp(rsdp)
            .dwarf_data(dwarf_data)
            .packed(packed)
            .kernel_elf(elf);

        builder.build()
    }
}

impl<T: AnyInitializationStage> InitializationContext<T> {
    pub fn start(config: TomlValue) -> InitializationContext<Stage0> {
        InitializationContext { context: Stage0 { config } }
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
