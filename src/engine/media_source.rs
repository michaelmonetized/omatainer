use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LibSource {
    Builtin(BuiltinStem),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BuiltinStem {
    Drums,
    Harmony,
}

impl BuiltinStem {
    pub fn index(self) -> u8 {
        match self {
            Self::Drums => 0,
            Self::Harmony => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub source: LibSource,
    pub title: String,
}
