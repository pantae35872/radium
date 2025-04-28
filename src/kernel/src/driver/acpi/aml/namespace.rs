use core::fmt::{self, Display};
use core::iter;
use core::str::FromStr;

use alloc::collections::btree_map::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use sentinel::log;

use super::AmlError;

#[derive(PartialEq, Clone, Copy, Debug, Eq, PartialOrd, Ord)]
pub struct NameSeg([char; 4]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmlName(pub Vec<NameComponent>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameComponent {
    Root,
    Prefix,
    Segment(NameSeg),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LevelType {
    Scope,
    Device,
    Processor,
    PowerResource,
    ThermalZone,
    MethodLocals,
}

#[derive(Clone, Debug)]
pub struct NamespaceLevel {
    pub typ: LevelType,
    pub children: BTreeMap<NameSeg, NamespaceLevel>,
    pub values: BTreeMap<NameSeg, AmlHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct AmlHandle(u32);

#[derive(Debug)]
pub struct Namespace {
    //object_map: BTreeMap<AmlHandle, AmlValue>,
    root: NamespaceLevel,
}

impl NamespaceLevel {
    pub fn new(typ: LevelType) -> NamespaceLevel {
        NamespaceLevel {
            typ,
            children: BTreeMap::new(),
            values: BTreeMap::new(),
        }
    }
}

impl Namespace {
    pub fn new() -> Self {
        Namespace {
            root: NamespaceLevel::new(LevelType::Scope),
        }
    }

    pub fn add_level(&mut self, path: AmlName, typ: LevelType) -> Result<(), AmlError> {
        assert!(path.is_absolute());
        let mut path = path.normalize()?;

        if path != AmlName::root() {
            let name = path.0.pop().unwrap().as_nameseg().unwrap();
            let level = self.get_level_from_path_mut(&path)?;
            level
                .children
                .entry(name)
                .and_modify(|level: &mut NamespaceLevel| {
                    log!(
                        Warning,
                        "Aml trying to create namespace level, to an existing one with type: {:?}, path: {path}, name: {name}, new_type: {typ:?}",
                        level.typ
                    )
                })
                .or_insert_with(|| NamespaceLevel::new(typ));
        }

        return Ok(());
    }

    pub fn get_level_from_path(&self, path: &AmlName) -> Result<&NamespaceLevel, AmlError> {
        assert!(path.is_absolute());

        if path == &AmlName::root() {
            return Ok(&self.root);
        }

        path.0
            .iter()
            .skip(1)
            .try_fold(&self.root, |current_level, name| {
                match current_level.children.get(
                    &name
                        .as_nameseg()
                        .map_err(|_| AmlError::PathIsNotNormalize)?,
                ) {
                    Some(new_level) => Ok(new_level),
                    None => Err(AmlError::LevelDoesNotExists { path: path.clone() }),
                }
            })
    }

    pub fn get_level_from_path_mut(
        &mut self,
        path: &AmlName,
    ) -> Result<&mut NamespaceLevel, AmlError> {
        assert!(path.is_absolute());

        if path == &AmlName::root() {
            return Ok(&mut self.root);
        }

        path.0
            .iter()
            .skip(1)
            .try_fold(&mut self.root, |current_level, name| {
                match current_level.children.get_mut(
                    &name
                        .as_nameseg()
                        .map_err(|_| AmlError::PathIsNotNormalize)?,
                ) {
                    Some(new_level) => Ok(new_level),
                    None => Err(AmlError::LevelDoesNotExists { path: path.clone() }),
                }
            })
    }
}

impl AmlName {
    pub fn null_name() -> Self {
        Self(Vec::new())
    }

    pub fn root() -> Self {
        Self(vec![NameComponent::Root])
    }

    pub fn is_normalize(&self) -> bool {
        !self.0.contains(&NameComponent::Prefix)
    }

    pub fn is_absolute(&self) -> bool {
        self.0
            .get(0)
            .is_some_and(|e| matches!(e, NameComponent::Root))
    }

    pub fn normalize(self) -> Result<AmlName, AmlError> {
        if !self.is_absolute() {
            log!(Warning, "Trying to normalize an relative aml path");
            return Ok(self);
        }
        if self.is_normalize() {
            return Ok(self);
        }

        self.0
            .iter()
            .try_fold(Vec::new(), |mut new_name, name| match name {
                segment @ NameComponent::Segment(_) | segment @ NameComponent::Root => {
                    new_name.push(*segment);
                    Ok(new_name)
                }
                NameComponent::Prefix => match new_name.pop() {
                    Some(NameComponent::Segment(_)) => Ok(new_name),
                    Some(NameComponent::Root) | None => Err(AmlError::NormalizingInvalidName),
                    Some(NameComponent::Prefix) => unreachable!(),
                },
            })
            .map(|e| AmlName(e))
    }

    pub fn resolve(&self, scope: &AmlName) -> Result<AmlName, AmlError> {
        assert!(scope.is_absolute());

        if self.is_absolute() {
            return Ok(self.clone());
        }

        let mut path = scope.clone();
        path.0.extend_from_slice(&self.0);
        path.normalize()
    }

    pub fn search_rules_apply(&self) -> bool {
        self.0
            .get(0)
            .is_some_and(|e| matches!(e, NameComponent::Segment(_)))
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

impl Display for NameSeg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in self.0 {
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl Display for AmlName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for component in &self.0 {
            match component {
                NameComponent::Segment(seg) => write!(f, "{seg}")?,
                NameComponent::Prefix => write!(f, "^")?,
                NameComponent::Root => write!(f, "\\")?,
            }
        }
        Ok(())
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

impl NameComponent {
    fn as_nameseg(self) -> Result<NameSeg, AmlError> {
        match self {
            NameComponent::Segment(seg) => Ok(seg),
            _ => Err(AmlError::NotANameSeg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test_case]
    pub fn namespace_level() {
        let mut namespace = Namespace::new();
        assert!(namespace
            .add_level(AmlName::from_str("\\").unwrap(), LevelType::Scope)
            .is_ok());
        assert!(namespace
            .add_level(AmlName::from_str("\\_SB_").unwrap(), LevelType::Scope)
            .is_ok());
        assert!(namespace
            .get_level_from_path(&AmlName::from_str("\\").unwrap())
            .is_ok());
        assert!(namespace
            .get_level_from_path(&AmlName::from_str("\\_SB_").unwrap())
            .is_ok());
        assert!(namespace
            .add_level(AmlName::from_str("\\_SB_.PCI1").unwrap(), LevelType::Device)
            .is_ok());
        assert!(namespace
            .add_level(
                AmlName::from_str("\\_SB_.^MINE").unwrap(),
                LevelType::ThermalZone
            )
            .is_ok());
        assert!(namespace
            .get_level_from_path(&AmlName::from_str("\\_SB_.PCI1").unwrap())
            .is_ok_and(|e| e.typ == LevelType::Device));
        assert!(namespace
            .get_level_from_path(&AmlName::from_str("\\MINE").unwrap())
            .is_ok_and(|e| e.typ == LevelType::ThermalZone));
    }

    #[test_case]
    pub fn aml_name_resolve() {
        assert_eq!(
            AmlName::from_str("_CRS")
                .unwrap()
                .resolve(&AmlName::from_str("\\_SB_.PCI0").unwrap()),
            Ok(AmlName::from_str("\\_SB_.PCI0._CRS").unwrap())
        );
        assert_eq!(
            AmlName::from_str("^_CRS")
                .unwrap()
                .resolve(&AmlName::from_str("\\_SB_.PCI0").unwrap()),
            Ok(AmlName::from_str("\\_SB_._CRS").unwrap())
        );
        assert_eq!(
            AmlName::from_str("^_CRS")
                .unwrap()
                .resolve(&AmlName::from_str("\\_SB_.PCI0.BUS0").unwrap()),
            Ok(AmlName::from_str("\\_SB_.PCI0._CRS").unwrap())
        );
        assert_eq!(
            AmlName::from_str("^_FOO.PCR_")
                .unwrap()
                .resolve(&AmlName::from_str("\\_SB_.PCI0.BUS0").unwrap()),
            Ok(AmlName::from_str("\\_SB_.PCI0._FOO.PCR_").unwrap())
        );
        assert_eq!(
            AmlName::from_str("\\_SB_._FOO.PCR_")
                .unwrap()
                .resolve(&AmlName::from_str("\\_SB_.PCI0.BUS0").unwrap()),
            Ok(AmlName::from_str("\\_SB_._FOO.PCR_").unwrap())
        );
        assert_eq!(
            AmlName::from_str("^^^^^^^_FOO.PCR_")
                .unwrap()
                .resolve(&AmlName::from_str("\\_SB_.PCI0.BUS0").unwrap()),
            Err(AmlError::NormalizingInvalidName)
        );
    }

    #[test_case]
    pub fn aml_name_normalize() {
        assert_eq!(
            AmlName::from_str("\\_SB_.PCI0.^FOO_")
                .unwrap()
                .is_normalize(),
            false
        );
        assert_eq!(
            AmlName::from_str("\\_SB_.^PCI0").unwrap().normalize(),
            Ok(AmlName::from_str("\\PCI0").unwrap())
        );
        assert_eq!(
            AmlName::from_str("\\_SB_.PCI1.FOO_.^^BAR1")
                .unwrap()
                .normalize(),
            Ok(AmlName::from_str("\\_SB_.BAR1").unwrap())
        );
        assert_eq!(
            AmlName::from_str("\\^^BAR1").unwrap().normalize(),
            Err(AmlError::NormalizingInvalidName)
        );
        assert_eq!(
            AmlName::from_str("\\^FOO_").unwrap().normalize(),
            Err(AmlError::NormalizingInvalidName)
        );
    }

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
