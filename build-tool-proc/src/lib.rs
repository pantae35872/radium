use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    DataEnum, DataStruct, DeriveInput, Error, Fields, Ident, Meta, MetaNameValue, parse_macro_input, spanned::Spanned,
};

#[proc_macro_derive(Config, attributes(config_name))]
pub fn config(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let input_span = input.span();
    let name = input.ident;
    match input.data {
        syn::Data::Enum(data) => gen_enum(name, data),
        syn::Data::Struct(data) => gen_struct(name, data),
        syn::Data::Union(_) => Err(Error::new(input_span, "Union config is not supported")),
    }
    .unwrap_or_else(syn::Error::into_compile_error)
    .into()
}

fn gen_struct(name: Ident, data: DataStruct) -> Result<TokenStream, Error> {
    let mut field_where_clause = Vec::new();
    let mut field_names = Vec::new();
    let mut field_config_names = Vec::new();
    let field_len = data.fields.len();
    for field in data.fields.iter() {
        let field_name = field.ident.as_ref().ok_or(Error::new(field.span(), "tuple structs is not supported"))?;

        let cfg_name =
            field.attrs.iter().find(|attr| attr.path().is_ident("config_name")).and_then(|attr| match &attr.meta {
                Meta::NameValue(MetaNameValue {
                    value: syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(cfg_name), .. }),
                    ..
                }) => Some(cfg_name),
                _ => None,
            });
        let cfg_name = cfg_name
            .map(|lit| lit.value())
            .ok_or(Error::new(field.span(), "This field doesn't have #[config_name = \"your cfg name\"] on it"))?;

        let field_type = &field.ty;
        field_where_clause.push(quote! {
            #field_type: Config
        });
        field_names.push(field_name.clone());
        field_config_names.push(cfg_name);
    }
    Ok(quote! {
        const _: () = {
            use crate::config::{Config, ConfigTree};

            #[automatically_derived]
            impl Config for #name
            where
                #(#field_where_clause,)*
            {
                fn into_tree(self, name: String) -> ConfigTree {
                    ConfigTree::Group { name, members: Into::<Vec<ConfigTree>>::into(self) }
                }
            }

            #[automatically_derived]
            impl TryFrom<Vec<ConfigTree>> for #name
            where
                #(#field_where_clause,)*
            {
                type Error = Vec<ConfigTree>;

                fn try_from(members: Vec<ConfigTree>) -> Result<Self, Self::Error> {
                    let [#(#field_names,)*] = TryInto::<[ConfigTree; #field_len]>::try_into(members)?;
                    Ok(Self { #(#field_names: TryFrom::try_from(#field_names).map_err(|err| vec![err])?,)* })
                }
            }

            #[automatically_derived]
            impl TryFrom<ConfigTree> for #name
            where
                #(#field_where_clause,)*
            {
                type Error = ConfigTree;
                fn try_from(value: ConfigTree) -> Result<Self, Self::Error> {
                    match value {
                        ConfigTree::Group { name, members, .. } => TryInto::<Self>::try_into(members)
                            .map_err(|error| ConfigTree::Group { name, members: error }),
                        t => Err(t),
                    }
                }
            }

            #[automatically_derived]
            impl From<#name> for Vec<ConfigTree>
            where
                #(#field_where_clause,)*
            {
                fn from(value: #name) -> Self {
                    vec![
                        #( value.#field_names.into_tree(#field_config_names.to_string()),)*
                    ]
                }
            }
        };
    })
}

fn gen_enum(name: Ident, data: DataEnum) -> Result<TokenStream, Error> {
    let mut try_from_tree_match = Vec::new();
    let mut from_self_match = Vec::new();
    let mut value_fields = Vec::new();
    for (i, variant) in data.variants.iter().enumerate() {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new(variant.span(), "Config variant can only be unit field"));
        }

        let variant_name = &variant.ident;
        try_from_tree_match.push(quote! {
            ConfigTree::Value { value: ConfigValue::Union { current: #i, .. }, .. } => Ok(#name::#variant_name)
        });

        from_self_match.push(quote! {
            #name::#variant_name => #i
        });

        let variant_str = variant_name.to_string();
        value_fields.push(quote! {
            #variant_str.to_string()
        });
    }

    Ok(quote! {
        const _: () = {
            use crate::config::{Config, ConfigTree, ConfigValue};

            #[automatically_derived]
            impl Config for #name {
                fn into_tree(self, name: String) -> ConfigTree {
                    ConfigTree::Value { name, value: Into::<ConfigValue>::into(self) }
                }
            }

            #[automatically_derived]
            impl TryFrom<ConfigTree> for #name {
                type Error = ConfigTree;

                fn try_from(value: ConfigTree) -> Result<Self, ConfigTree> {
                    match value {
                        #(#try_from_tree_match,)*
                        t => Err(t),
                    }
                }
            }

            #[automatically_derived]
            impl From<#name> for ConfigValue {
                fn from(value: #name) -> Self {
                    let current = match value {
                        #(#from_self_match,)*
                    };
                    ConfigValue::Union {
                        current,
                        values: vec![#(#value_fields),*]
                    }
                }
            }
        };
    })
}
