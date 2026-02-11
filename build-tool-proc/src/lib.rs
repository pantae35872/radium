use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
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
    let name_str_lit = name.to_string();

    let mut copied_field = String::new();
    let mut copied_value_get = Vec::new();
    let mut copied_value = String::new();
    let mut type_gen = Vec::new();

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

        copied_field.push_str(&format!(
            "    pub {}: {},\n",
            field_name.to_token_stream(),
            // TODO: Hard coded for now
            match field_type.to_token_stream().to_string() {
                t if &t == "String" => "&'static str".to_string(),
                t => t,
            }
        ));

        copied_value_get.push(quote!(
            {
                let string = self.#field_name.into_const_rust();
                let mut lines = string.lines();

                let mut result = String::new();

                if let Some(first) = lines.next() {
                    result.push_str(first);
                }

                for line in lines {
                    result.push('\n');
                    result.push_str("    ");
                    result.push_str(line);
                }

                result
            }
        ));
        copied_value.push_str(&format!("\n    {}: {{}},", field_name.to_token_stream()));
        type_gen.push(quote! {
            accum.push_str(&self.#field_name.into_const_rust_types());
        });
    }

    let rust_const = format! {
        "{name} {{{{{copied_value}\n}}}}"
    };
    Ok(quote! {
        const _: () = {
            use crate::config::{Config, ConfigTree, Error};

            #[automatically_derived]
            impl Config for #name
            where
                #(#field_where_clause,)*
            {
                fn into_tree(self, name: String) -> ConfigTree {
                    ConfigTree::Group { name, members: Into::<Vec<ConfigTree>>::into(self) }
                }

                fn modifier_config<'a, C: IntoIterator<Item = &'a str>>(&mut self, config: C, value: &str) -> Result<(), Error> {
                    let mut config = config.into_iter();
                    match config.next().ok_or(Error::CannotModifyWholeGroup)? {
                        #( stringify!(#field_names) => self.#field_names.modifier_config(config, value)?,)*
                        unknown => return Err(Error::UnknownConfig(unknown.to_string())),
                    };
                    Ok(())
                }

                fn into_const_rust(&self) -> String {
                    format!(
                        #rust_const,
                        #(#copied_value_get,)*
                    )
                }

                fn into_const_rust_types(&self) -> String {
                    let mut accum = String::new();
                    accum.push_str(&format!("\n#[derive(Debug)]\npub struct {name} {{\n{copied_field}}}\n", name = #name_str_lit, copied_field = #copied_field));
                    #(#type_gen)*
                    accum
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
    let mut from_overwrite_match = Vec::new();
    let mut value_fields = Vec::new();

    let mut match_gen = Vec::new();
    let mut copied_variant = String::new();

    let name_str_lit = name.to_string();
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

        let variant_str_snake = variant_str.to_case(Case::Snake);
        from_overwrite_match.push(quote! {
            #variant_str_snake => #name::#variant_name
        });

        let value = format!("{}::{}", name.to_token_stream(), variant_name.to_token_stream());
        match_gen.push(quote! {
            #name::#variant_name => #value.to_string()
        });

        copied_variant.push_str(&format!("    {},\n", variant_str));
    }

    Ok(quote! {
        const _: () = {
            use crate::config::{Config, ConfigTree, ConfigValue};

            #[automatically_derived]
            impl Config for #name {
                fn into_tree(self, name: String) -> ConfigTree {
                    ConfigTree::Value { name, value: Into::<ConfigValue>::into(self) }
                }

                fn into_const_rust(&self) -> String {
                    match self {
                        #(#match_gen,)*
                    }
                }

                fn modifier_config<'a, C: IntoIterator<Item = &'a str>>(&mut self, config: C, value: &str) -> Result<(), Error> {
                    *self = match value {
                        #(#from_overwrite_match,)*
                        invalid => return Err(Error::InvalidValue(invalid.to_string())),
                    };
                    Ok(())
                }

                fn into_const_rust_types(&self) -> String {
                    format!("\n#[derive(Debug)]\npub enum {name} {{\n{copied_variant}}}\n", name = #name_str_lit, copied_variant = #copied_variant)
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
