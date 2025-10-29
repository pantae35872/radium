#![feature(trim_prefix_suffix)]

use core::str;
use std::{
    env,
    fs::{OpenOptions, remove_file},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use proc_macro::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Expr, Ident, Token, Type, parenthesized,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    token::Paren,
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
    path: &Path,
    full_name: &Ident,
    ty: &str,
    initial: &str,
    append: &str,
    append_index: u64,
) {
    let mut wrote: String = "".to_string();
    let build_uuid = env::var("BUILD_UUID").unwrap();
    let mut length = 0;
    if let Ok(mut ok) = OpenOptions::new().read(true).open(path) {
        length = ok.seek(SeekFrom::End(0)).unwrap();
    }
    if let Ok(mut ok) = OpenOptions::new().read(true).open(path) {
        ok.read_to_string(&mut wrote).unwrap();
    }

    if wrote
        .lines()
        .next()
        .is_some_and(|e| e.strip_prefix("//").is_some_and(|e| e != build_uuid))
        || length == 0
    {
        if path.exists() {
            remove_file(path).unwrap();
        }
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .unwrap()
            .write_all(format!("//{build_uuid}\n{initial}").as_bytes())
            .unwrap();
    }

    wrote.clear();

    if let Ok(mut ok) = OpenOptions::new().read(true).open(path) {
        ok.read_to_string(&mut wrote).unwrap();
    }

    if let Ok(mut ok) = OpenOptions::new().read(true).open(path) {
        length = ok.seek(SeekFrom::End(0)).unwrap();
    }

    let mut hash = DefaultHasher::new();
    full_name.hash(&mut hash);
    ty.hash(&mut hash);
    let hash = hash.finish().to_string();
    let should_write = !wrote
        .lines()
        .any(|e| e.trim().strip_prefix("//").is_some_and(|e| e == hash));

    if should_write {
        OpenOptions::new()
            .write(true)
            .open(path)
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

fn write_cpu_local_builder_set(path: &Path, full_name: &Ident, ty: &str) {
    tracked_write_file(
        path,
        full_name,
        ty,
        "#[allow(non_snake_case)]\nimpl CpuLocalBuilder2 { pub fn new() -> Self {Self::default()}\n\n}",
        &format!(
            "pub fn {full_name}(&mut self, value: {ty}) {{ self.{full_name} = Some(value); }}"
        ),
        1,
    );
}

fn write_cpu_local_builder_build(path: &Path, full_name: &Ident, ty: &str) {
    tracked_write_file(
        path,
        full_name,
        ty,
        "#[allow(non_snake_case)]\nimpl CpuLocalBuilder2 {\
            pub fn build(self) -> Option<CpuLocal2> {\
                Some(CpuLocal2 {\n\
                })\
            }\
        }",
        &format!("{full_name}: self.{full_name}?,"),
        4,
    );
}

fn write_cpu_local_builder_struct(path: &Path, full_name: &Ident, ty: &str) {
    tracked_write_file(
        path,
        full_name,
        ty,
        "#[allow(non_snake_case)]\n#[derive(Default)]\npub struct CpuLocalBuilder2 {\n}",
        &format!("pub {full_name}: Option<{ty}>,"),
        1,
    );
}

fn write_cpu_local(path: &Path, full_name: &Ident, ty: &str) {
    tracked_write_file(
        path,
        full_name,
        ty,
        "#[allow(non_snake_case)]\npub struct CpuLocal2 {\n}",
        &format!("pub {full_name}: {ty},"),
        1,
    );
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

    let local_path = out_dir.join("local_gen_struct.rs");
    let builder_struct = out_dir.join("local_gen_builder_struct.rs");
    let builder_build = out_dir.join("local_gen_builder_build.rs");
    let builder_set = out_dir.join("local_gen_builder_set.rs");

    let str_type = quote! { #ty }.to_string();

    write_cpu_local(&local_path, &full_name, &str_type);
    write_cpu_local_builder_struct(&builder_struct, &full_name, &str_type);
    write_cpu_local_builder_build(&builder_build, &full_name, &str_type);
    write_cpu_local_builder_set(&builder_set, &full_name, &str_type);

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
                &crate::smp::cpu_local2().#full_name
            }

            pub fn inner_mut(&self) -> &'static mut #ty {
                &mut crate::smp::cpu_local2().#full_name
            }
        }

        #vis static #name: #access_type = #access_type;
    }
    .into()
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
