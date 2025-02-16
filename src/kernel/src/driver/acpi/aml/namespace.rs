use core::iter;
use core::str::FromStr;

use alloc::vec::Vec;

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct NameSeg([char; 4]);

#[derive(Debug, Clone, PartialEq)]
pub struct AmlName(pub Vec<NameComponent>);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NameComponent {
    Root,
    Prefix,
    Segment(NameSeg),
}

pub struct Namespace {}

impl AmlName {
    pub fn null_name() -> Self {
        Self(Vec::new())
    }
}

impl FromStr for AmlName {
    type Err = ();

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let mut parse_name = Vec::new();
        let mut name = name.chars().peekable();
        if name.peek().is_some_and(|e| *e == '\\') {
            parse_name.push(NameComponent::Root);
            name.next();
        }

        parse_name
            .extend(iter::from_fn(|| name.next_if(|&e| e == '^')).map(|_| NameComponent::Prefix));
        let mut failed = false;
        while let Ok(segment) = name.next_chunk::<4>().inspect_err(|e| {
            if e.as_slice() != &[] {
                failed = true;
            }
        }) {
            parse_name.push(NameSeg::new(segment).ok_or(())?.into());
            if let None = name.next_if(|e| *e == '.') {
                break;
            }
            parse_name.extend(
                iter::from_fn(|| name.next_if(|&e| e == '^')).map(|_| NameComponent::Prefix),
            );
        }

        if name.next().is_some() || failed {
            return Err(());
        }

        Ok(Self(parse_name))
    }
}

impl NameSeg {
    pub fn new(name: [char; 4]) -> Option<Self> {
        Self::try_from(name.map(|e| e as u8)).ok()
    }

    pub fn new_bytes(name: [u8; 4]) -> Option<Self> {
        Self::try_from(name).ok()
    }
}

impl TryFrom<[char; 4]> for NameSeg {
    type Error = ();
    fn try_from(value: [char; 4]) -> Result<Self, Self::Error> {
        Self::try_from(value.map(|e| e as u8))
    }
}

impl TryFrom<[u8; 4]> for NameSeg {
    type Error = ();
    fn try_from(name: [u8; 4]) -> Result<Self, Self::Error> {
        match name {
            name if ((b'A'..=b'Z').contains(&name[0]) || name[0] == b'_')
                && name[1..4].iter().all(|e| {
                    (b'A'..=b'Z').contains(e) | (b'0'..=b'9').contains(e) | (*e == b'_')
                }) =>
            {
                Ok(Self(name.map(|e| e as char)))
            }
            _ => Err(()),
        }
    }
}

impl Into<NameComponent> for NameSeg {
    fn into(self) -> NameComponent {
        NameComponent::Segment(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test_case]
    pub fn aml_name_from_str() {
        assert_eq!(
            AmlName::from_str("\\_SB_.PCI0"),
            Ok(AmlName(vec![
                NameComponent::Root,
                NameSeg::new(['_', 'S', 'B', '_']).unwrap().into(),
                NameSeg::new(['P', 'C', 'I', '0']).unwrap().into()
            ]))
        );
        assert_eq!(
            AmlName::from_str("_SB_.^^PCI0"),
            Ok(AmlName(vec![
                NameSeg::new(['_', 'S', 'B', '_']).unwrap().into(),
                NameComponent::Prefix,
                NameComponent::Prefix,
                NameSeg::new(['P', 'C', 'I', '0']).unwrap().into(),
            ]))
        );
        assert_eq!(
            AmlName::from_str("_SB_.PCI0.SSEE"),
            Ok(AmlName(vec![
                NameSeg::new(['_', 'S', 'B', '_']).unwrap().into(),
                NameSeg::new(['P', 'C', 'I', '0']).unwrap().into(),
                NameSeg::new(['S', 'S', 'E', 'E']).unwrap().into()
            ]))
        );
        assert_eq!(
            AmlName::from_str("\\^^^_PR_"),
            Ok(AmlName(vec![
                NameComponent::Root,
                NameComponent::Prefix,
                NameComponent::Prefix,
                NameComponent::Prefix,
                NameSeg::new(['_', 'P', 'R', '_']).unwrap().into(),
            ]))
        );
        assert_eq!(AmlName::from_str("_SB_.PCI0^^"), Err(()));
        assert_eq!(AmlName::from_str("_SB_.A."), Err(()));
        assert_eq!(AmlName::from_str("^^\\_SB_.A."), Err(()));
    }
}
