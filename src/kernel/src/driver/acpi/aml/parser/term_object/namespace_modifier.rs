use crate::driver::acpi::aml::{
    namespace::LevelType,
    parser::{
        choose,
        name_string::name_string,
        opcode::{opcode, DEF_ALIAS, DEF_NAME, DEF_SCOPE},
        package_length::package_length,
        pair, try_with_context, Parser,
    },
};

use super::term_list;

pub fn namespace_modifier<'a, 'c>() -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    choose!(def_alias(), def_name(), def_scope())
}

fn def_alias<'a, 'c>() -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    opcode(DEF_ALIAS)
}

fn def_name<'a, 'c>() -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    opcode(DEF_NAME)
}

fn def_scope<'a, 'c>() -> impl Parser<'a, 'c, ()>
where
    'c: 'a,
{
    opcode(DEF_SCOPE).then(
        pair(package_length(), name_string())
            .map_with_context(|(length, name), context| {
                let previous_scope = context.current_scope.clone();
                context.current_scope =
                    try_with_context!(context, name.resolve(&context.current_scope));
                try_with_context!(
                    context,
                    context
                        .namespace
                        .add_level(context.current_scope.clone(), LevelType::Scope)
                );
                (Ok((length, previous_scope)), context)
            })
            .and_then(move |(length, previous_scope)| {
                term_list(length).map(move |_| previous_scope.clone())
            })
            .map_with_context(|previous_scope, context| {
                context.current_scope = previous_scope;
                (Ok(()), context)
            })
            .arced(),
    )
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use crate::driver::acpi::aml::{
        namespace::AmlName,
        parser::{opcode::DEF_SCOPE, parser_ok, Parser},
        AmlContext,
    };

    use super::def_scope;

    #[test_case]
    fn def_scope() {
        let mut context = AmlContext::test_context();

        parser_ok!(
            def_scope(),
            [DEF_SCOPE, 6, b'\\', 0, DEF_SCOPE, 0, b'_', b'S', b'B', b'_'],
            &mut context
        );

        assert!(context
            .namespace
            .get_level_from_path(&AmlName::from_str("\\_SB_").unwrap())
            .is_ok());
    }
}
