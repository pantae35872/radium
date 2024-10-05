use std::sync::Mutex;

use quote::quote;
use syn::{parse_macro_input, LitInt};

const BASE_ADDR: u64 = 0xFFFFFFFF00000000;
const ALIGN: u64 = 4096;
static NEXT_ADDR: Mutex<u64> = Mutex::new(BASE_ADDR);

#[proc_macro]
pub fn comptime_alloc(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let size = parse_macro_input!(input as LitInt);
    let size = size.base10_parse::<u64>().expect("Failed to parse size");
    let mut next_addr = NEXT_ADDR.lock().expect("Failed to allocate");
    let addr = *next_addr;
    *next_addr += size + (size as *const u8).align_offset(ALIGN as usize) as u64;

    let expanded = quote! {
        #addr
    };
 
    expanded.into()
}
