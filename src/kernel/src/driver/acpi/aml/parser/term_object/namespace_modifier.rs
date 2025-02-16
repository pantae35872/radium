use crate::{
    choose,
    driver::acpi::aml::parser::{
        opcode::{opcode, DEF_ALIAS, DEF_NAME, DEF_SCOPE},
        Parser,
    },
};

fn namespace_modifier<'a>() -> impl Parser<'a, ()> {
    choose!(def_alias(), def_name())
}

fn def_alias<'a>() -> impl Parser<'a, ()> {
    opcode(DEF_ALIAS)
}

fn def_name<'a>() -> impl Parser<'a, ()> {
    opcode(DEF_NAME)
}

fn def_scope<'a>() -> impl Parser<'a, ()> {
    opcode(DEF_SCOPE)
}
