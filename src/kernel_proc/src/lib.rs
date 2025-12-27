#![feature(trim_prefix_suffix)]

use core::str;
use std::{
    env,
    fs::{OpenOptions, remove_file},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::FileExt,
    path::PathBuf,
};

use convert_case::{Case, Casing};
use proc_macro::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Expr, Ident, ItemStruct, Token, Type, parenthesized, parse::{Parse, ParseStream}, parse_macro_input, punctuated::Punctuated, token::Paren
};

#[proc_macro]
pub fn fill_idt(_item: TokenStream) -> TokenStream {
    let handlers = (32..255).map(|interrupt_number| {
        let interrupt_number = interrupt_number as usize;
        let fn_name = format_ident!("gen_interrupt_{}", interrupt_number);
        quote! {
            unsafe { idt[#interrupt_number]
                .set_handler_addr(VirtAddr::new(#fn_name as u64))
                .set_stack_index(GENERAL_STACK_INDEX) };
        }
    });
    let output = quote! {
        #(#handlers)*
    };
    output.into()
}

#[proc_macro]
pub fn generate_interrupt_handlers(_item: TokenStream) -> TokenStream {
    let handlers = (32..255)
        .map(|interrupt_number| {
            let interrupt_number = interrupt_number as usize;
            let fn_name = format_ident!("gen_interrupt_{}", interrupt_number);
            let interrupt_number_asm = format!("mov rsi, {interrupt_number}");
            quote! {
                #[unsafe(no_mangle)]
                #[unsafe(naked)]
                extern "C" fn #fn_name() {
                    unsafe {
                        core::arch::naked_asm!(
                            // Save general-purpose registers
                            "push rax",
                            "push rbx",
                            "push rcx",
                            "push rdx",
                            "push rbp",
                            "push rdi",
                            "push rsi",
                            "push r8",
                            "push r9",
                            "push r10",
                            "push r11",
                            "push r12",
                            "push r13",
                            "push r14",
                            "push r15",
                            "mov rdi, rsp",
                            #interrupt_number_asm,
                            "call external_interrupt_handler",
                            "pop r15",
                            "pop r14",
                            "pop r13",
                            "pop r12",
                            "pop r11",
                            "pop r10",
                            "pop r9",
                            "pop r8",
                            "pop rsi",
                            "pop rdi",
                            "pop rbp",
                            "pop rdx",
                            "pop rcx",
                            "pop rbx",
                            "pop rax",
                            // Return from interrupt
                            "iretq",
                        );
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let output = quote! {
        #(#handlers)*
    };
    output.into()
}

#[proc_macro]
pub fn local_gen(_input: TokenStream) -> TokenStream {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let local_path = out_dir.join("local_gen_struct.rs").display().to_string();
    let builder_struct = out_dir
        .join("local_gen_builder_struct.rs")
        .display()
        .to_string();
    let builder_build = out_dir
        .join("local_gen_builder_build.rs")
        .display()
        .to_string();
    let builder_set = out_dir
        .join("local_gen_builder_set.rs")
        .display()
        .to_string();

    quote! {
        include!(#local_path);
        include!(#builder_struct);
        include!(#builder_build);
        include!(#builder_set);
    }
    .into()
}

struct Builder {
    builder: Ident,
    _comma: Token![,],
    builds: Punctuated<BuilderBuild, Token![,]>,
}

impl Parse for Builder {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            builder: input.parse()?,
            _comma: input.parse()?,
            builds: input.parse_terminated(BuilderBuild::parse, Token![,])?,
        })
    }
}

struct BuilderBuild {
    name: Ident,
    _paren: Paren,
    create: Expr,
}

impl Parse for BuilderBuild {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let create_expr;
        Ok(Self {
            name: input.parse()?,
            _paren: parenthesized!(create_expr in input),
            create: create_expr.parse()?,
        })
    }
}

impl ToTokens for BuilderBuild {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let name = &self.name;
        let create = &self.create;
        quote! {
            #name(#create)
        }
        .to_tokens(tokens);
    }
}

#[proc_macro]
pub fn local_builder(input: TokenStream) -> TokenStream {
    let Builder {
        builder,
        mut builds,
        ..
    } = parse_macro_input!(input as Builder);
    let module = Span::call_site().file().replace("/", "_");
    let module = module.trim_suffix(".rs");
    for build in builds.iter_mut() {
        build.name = format_ident!("__{module}_{}", build.name);
    }
    let builds = builds.iter();

    quote! {
        #(#builder.#builds);*
    }
    .into()
}

fn tracked_write_file(
    name: &str,
    hash: impl FnOnce(&mut DefaultHasher),
    initial: &str,
    append: &str,
    append_index: u64,
) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    assert!(out_dir.exists());

    let path = out_dir.join(name).with_extension("rs");

    let mut wrote: String = "".to_string();
    let build_uuid = env::var("BUILD_UUID").unwrap();
    let mut length = 0;
    if let Ok(mut ok) = OpenOptions::new().read(true).open(&path) {
        length = ok.seek(SeekFrom::End(0)).unwrap();
    }
    if let Ok(mut ok) = OpenOptions::new().read(true).open(&path) {
        ok.read_to_string(&mut wrote).unwrap();
    }

    if wrote
        .lines()
        .next()
        .is_some_and(|e| e.strip_prefix("//").is_some_and(|e| e != build_uuid))
        || length == 0
    {
        if path.exists() {
            remove_file(&path).unwrap();
        }
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .unwrap()
            .write_all(format!("//{build_uuid}\n{initial}").as_bytes())
            .unwrap();
    }

    wrote.clear();

    if let Ok(mut ok) = OpenOptions::new().read(true).open(&path) {
        ok.read_to_string(&mut wrote).unwrap();
    }

    if let Ok(mut ok) = OpenOptions::new().read(true).open(&path) {
        length = ok.seek(SeekFrom::End(0)).unwrap();
    }

    let mut hasher = DefaultHasher::new();
    hash(&mut hasher);
    let hash = hasher.finish().to_string();
    let should_write = !wrote
        .lines()
        .any(|e| e.trim().strip_prefix("//").is_some_and(|e| e == hash));

    if should_write {
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .write_all_at(
                format!(
                    "//{hash}\n{append}\n{}",
                    &wrote[wrote.len() - append_index as usize..]
                )
                .as_bytes(),
                length - append_index,
            )
            .unwrap();
    }
}

#[proc_macro]
pub fn def_local(input: TokenStream) -> TokenStream {
    let Def { vis, name, ty, .. } = parse_macro_input!(input as Def);
    let module = Span::call_site().file().replace("/", "_");
    let module = module.trim_suffix(".rs");
    let full_name = format_ident!("__{module}_{name}");
    let access_type = format_ident!("__Access{full_name}");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    assert!(out_dir.exists());

    let str_type = quote! { #ty }.to_string();
    let hasher = |hasher: &mut DefaultHasher| {
        full_name.hash(hasher);
        str_type.hash(hasher);
    };

    tracked_write_file(
        "local_gen_struct",
        hasher,
        "#[allow(non_snake_case)]\npub struct CpuLocal {\n}",
        &format!("pub {full_name}: {str_type},"),
        1,
    );

    tracked_write_file(
        "local_gen_builder_struct",
        hasher,
        "#[allow(non_snake_case)]\n#[derive(Default)]\npub struct CpuLocalBuilder {\n}",
        &format!("pub {full_name}: Option<{str_type}>,"),
        1,
    );

    tracked_write_file(
        "local_gen_builder_build",
        hasher,
        "#[allow(non_snake_case)]\nimpl CpuLocalBuilder {\
            pub fn build(self) -> Option<CpuLocal> {\
                Some(CpuLocal {\n\
                })\
            }\
        }",
        &format!("{full_name}: self.{full_name}?,"),
        4,
    );

    tracked_write_file(
        "local_gen_builder_set",
        hasher,
        "#[allow(non_snake_case)]\nimpl CpuLocalBuilder { pub fn new() -> Self {Self::default()}\n\n}",
        &format!(
            "pub fn {full_name}(&mut self, value: {str_type}) {{ self.{full_name} = Some(value); }}"
        ),
        1,
    );

    quote! {
        #vis struct #access_type;

        impl core::ops::Deref for #access_type {
            type Target = #ty;

            fn deref(&self) -> &Self::Target {
                self.inner()
            }
        }

        impl #access_type {
            pub fn inner(&self) -> &'static #ty {
                &crate::smp::cpu_local().#full_name
            }

            pub fn inner_mut(&self) -> &'static mut #ty {
                &mut crate::smp::cpu_local().#full_name
            }
        }

        #vis static #name: #access_type = #access_type;
    }
    .into()
}

#[proc_macro]
pub fn gen_ipp(_item: TokenStream) -> TokenStream {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let constants_path = out_dir.join("ipp_gen_constants.rs").display().to_string();
    let pipeline_path = out_dir
        .join("ipp_gen_packet_pipeline.rs")
        .display()
        .to_string();

    let packets_path = out_dir.join("ipp_gen_packets.rs").display().to_string();
    let impl_path = out_dir.join("ipp_gen_packet_impl.rs").display().to_string();

    quote! {
        include!(#constants_path);
        include!(#pipeline_path);
        include!(#packets_path);
        include!(#impl_path);
    }
    .into()
}

#[proc_macro_derive(IPPacket)]
pub fn ipp_packet(input: TokenStream) -> TokenStream {
    let packet = parse_macro_input!(input as ItemStruct);
    let ty = &packet.ident;
    let upper_case_name = packet.ident.to_string().to_case(Case::Constant);
    let static_packet_name = format_ident!("{upper_case_name}_PACKETS");
    let static_handled_flags_name = format_ident!("{upper_case_name}_HANDLED_FLAGS",);

    quote! {
        static #static_packet_name: [crate::sync::spin_mpsc::SpinMPSC<#ty, 256>; crate::smp::MAX_CPU] = [
            const { crate::sync::spin_mpsc::SpinMPSC::new() }; crate::smp::MAX_CPU];
        static #static_handled_flags_name: [core::sync::atomic::AtomicBool; crate::smp::MAX_CPU] = 
            [const { core::sync::atomic::AtomicBool::new(false) }; crate::smp::MAX_CPU];

        impl #ty {
            pub fn broadcast(self, urgent: bool)
            where
                for<'a> Self: Clone,
            {
                for flag in #static_handled_flags_name.iter() {
                    flag.store(false, core::sync::atomic::Ordering::Release);
                }

                for (core, packet) in #static_packet_name 
                    .iter()
                    .enumerate()
                    .filter(|(core, ..)| crate::interrupt::CORE_ID.id() != *core && *core < *crate::smp::CORE_COUNT)
                {
                    let mut send = self.clone();
                    while let Err(failed) = packet.push(send) {
                        crate::interrupt::LAPIC.inner_mut().send_fixed_ipi(
                            crate::smp::CoreId::new(core).unwrap(),
                            crate::interrupt::InterruptIndex::CheckIPP,
                        );

                        send = failed;
                    }
                }

                crate::interrupt::LAPIC
                    .inner_mut()
                    .broadcast_fixed_ipi(crate::interrupt::InterruptIndex::CheckIPP);

                if urgent {
                    loop {
                        let cores = #static_handled_flags_name
                            .iter()
                            .enumerate()
                            .filter(|(core, ..)| crate::interrupt::CORE_ID.id() != *core && *core < *crate::smp::CORE_COUNT)
                            .map(|(core, flag)| {
                                (
                                    crate::smp::CoreId::new(core).unwrap(),
                                    flag.load(core::sync::atomic::Ordering::Acquire),
                                )
                            })
                            .filter_map(|(core, handled)| crate::inline_if!(handled, None, Some(core)));
                        let mut all_handled = true;

                        for core in cores {
                            all_handled = false;
                            crate::interrupt::LAPIC
                                .inner_mut()
                                .send_fixed_ipi(core, crate::interrupt::InterruptIndex::CheckIPP);
                        }

                        if all_handled {
                            break;
                        }
                    }
                }
            }

            pub fn send(self, core_id: crate::smp::CoreId, urgent: bool) {
                let core = core_id.id();
                #static_handled_flags_name[core].store(false, core::sync::atomic::Ordering::Release);
                let mut send = self;
                while let Err(failed) = #static_packet_name[core].push(send) {
                    crate::interrupt::LAPIC
                        .inner_mut()
                        .send_fixed_ipi(core_id, crate::interrupt::InterruptIndex::CheckIPP);

                    send = failed;
                }

                crate::interrupt::LAPIC
                    .inner_mut()
                    .send_fixed_ipi(core_id, crate::interrupt::InterruptIndex::CheckIPP);

                if urgent {
                    while !#static_handled_flags_name[core].load(core::sync::atomic::Ordering::Acquire) {
                        crate::interrupt::LAPIC
                            .inner_mut()
                            .send_fixed_ipi(core_id, crate::interrupt::InterruptIndex::CheckIPP);
                    }
                }
            }

            fn handle(mut handler: impl FnMut(#ty)) {
                while let Some(c) = #static_packet_name[crate::interrupt::CORE_ID.id()].pop() {
                    handler(c)
                }

                #static_handled_flags_name[crate::interrupt::CORE_ID.id()].store(true, core::sync::atomic::Ordering::Release);
            }
        }
    }.into()
}

struct Def {
    vis: Option<Token![pub]>,
    _static: Token![static],
    name: Ident,
    _colon: Token![:],
    ty: Type,
}

impl Parse for Def {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            vis: input.parse()?,
            _static: input.parse()?,
            name: input.parse()?,
            _colon: input.parse()?,
            ty: input.parse()?,
        })
    }
}
