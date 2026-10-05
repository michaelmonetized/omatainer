//! Saved library presentation belongs to the active user profile.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Column {
    Title,
    Bpm,
    Key,
    Length,
    Played,
    Artist,
    Rating,
    Color,
    Group,
    Tags,
    Notes,
}
impl Column {
    pub const ALL: [Self; 11] = [
        Self::Title,
        Self::Bpm,
        Self::Key,
        Self::Length,
        Self::Played,
        Self::Artist,
        Self::Rating,
        Self::Color,
        Self::Group,
        Self::Tags,
        Self::Notes,
    ];
    /// Name a metadata column.
    /// Takes this column; returns its fixed visible name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Bpm => "BPM",
            Self::Key => "Key",
            Self::Length => "Length",
            Self::Played => "Last play",
            Self::Artist => "Artist",
            Self::Rating => "Rating",
            Self::Color => "Color",
            Self::Group => "Group",
            Self::Tags => "Tags",
            Self::Notes => "Notes",
        }
    }
    /// Choose the original column width.
    /// Takes this column; returns its unscaled width in points.
    fn width(self) -> f32 {
        match self {
            Self::Title => 280.0,
            Self::Bpm => 112.0,
            Self::Key => 48.0,
            Self::Length => 64.0,
            Self::Played => 140.0,
            Self::Artist => 180.0,
            Self::Rating => 64.0,
            Self::Color => 88.0,
            Self::Group => 128.0,
            Self::Tags => 160.0,
            Self::Notes => 240.0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ColumnSpec {
    pub column: Column,
    pub visible: bool,
    pub width: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Density {
    Compact,
    Comfortable,
    Artwork,
}
impl Density {
    /// Name a row density.
    /// Takes this density; returns a fixed visible name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Artwork => "Artwork",
        }
    }
    /// Size a row before display scaling.
    /// Takes this density; returns its minimum height in points.
    pub fn height(self) -> f32 {
        match self {
            Self::Compact => 18.0,
            Self::Comfortable => 30.0,
            Self::Artwork => 72.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sort {
    pub column: Column,
    pub descending: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Layout {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    pub density: Density,
    pub primary: Option<Sort>,
    pub secondary: Option<Sort>,
}
impl Layout {
    /// Create a complete column layout.
    /// Takes a name and row density; returns all eleven columns in original order with manual sorting.
    pub fn new(name: &str, density: Density) -> Self {
        Self {
            name: name.into(),
            columns: Column::ALL
                .into_iter()
                .map(|column| ColumnSpec {
                    column,
                    visible: true,
                    width: column.width(),
                })
                .collect(),
            density,
            primary: None,
            secondary: None,
        }
    }
    /// Bound saved presentation and reject ambiguous sorts.
    /// Takes this layout; returns an error for incomplete columns, hidden content or invalid dimensions.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.len() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Layout names need 1–80 visible bytes without surrounding spaces".into());
        }
        if self.columns.len() != Column::ALL.len()
            || Column::ALL.iter().any(|column| {
                self.columns
                    .iter()
                    .filter(|spec| spec.column == *column)
                    .count()
                    != 1
            })
            || !self.columns.iter().any(|spec| spec.visible)
        {
            return Err("Keep every column once and at least one visible column".into());
        }
        if self
            .columns
            .iter()
            .any(|spec| !spec.width.is_finite() || !(36.0..=1024.0).contains(&spec.width))
        {
            return Err("Column widths must be 36–1024 points".into());
        }
        if self.primary.is_none() && self.secondary.is_some()
            || self
                .primary
                .zip(self.secondary)
                .is_some_and(|(a, b)| a.column == b.column)
        {
            return Err("A secondary sort needs a different primary column".into());
        }
        Ok(())
    }
    /// Capture only the ordering fields.
    /// Takes this layout; returns primary and secondary sorts independent of widths and density.
    pub fn sorts(&self) -> [Option<Sort>; 2] {
        [self.primary, self.secondary]
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub active: String,
    pub layouts: Vec<Layout>,
}
impl Default for Config {
    fn default() -> Self {
        let mut artwork = Layout::new("Artwork", Density::Artwork);
        for spec in &mut artwork.columns {
            spec.visible = matches!(
                spec.column,
                Column::Title | Column::Artist | Column::Bpm | Column::Key | Column::Length
            );
        }
        Self {
            active: "Compact".into(),
            layouts: vec![
                Layout::new("Compact", Density::Compact),
                Layout::new("Comfortable", Density::Comfortable),
                artwork,
            ],
        }
    }
}
impl Config {
    /// Resolve the active saved layout.
    /// Takes validated configuration; returns its exact named layout.
    pub fn current(&self) -> &Layout {
        self.layouts
            .iter()
            .find(|layout| layout.name == self.active)
            .expect("validated library layout")
    }
    /// Edit the exact active layout.
    /// Takes validated configuration; returns its mutable named layout.
    pub fn current_mut(&mut self) -> &mut Layout {
        self.layouts
            .iter_mut()
            .find(|layout| layout.name == self.active)
            .expect("validated library layout")
    }
    /// Bound named layouts before save or application.
    /// Takes this configuration; returns success for at most sixteen distinct complete layouts.
    pub fn validate(&self) -> Result<(), String> {
        if self.layouts.is_empty()
            || self.layouts.len() > 16
            || !self.layouts.iter().any(|layout| layout.name == self.active)
        {
            return Err("Keep 1–16 layouts and select an existing layout".into());
        }
        let mut names = std::collections::HashSet::new();
        for layout in &self.layouts {
            layout.validate()?;
            if !names.insert(crate::localization::search_key(&layout.name)) {
                return Err("Layout names must be unique ignoring Unicode case".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::{storage, Preferences};
    use std::{path::Path, sync::atomic::AtomicBool};
    #[test]
    fn saved_layouts_reject_duplicate_columns_names_invalid_widths_and_hidden_content() {
        let valid = Config::default();
        valid.validate().unwrap();
        for edit in 0..7 {
            let mut invalid = valid.clone();
            let layout = &mut invalid.layouts[0];
            match edit {
                0 => layout.columns[0].column = Column::Bpm,
                1 => layout
                    .columns
                    .iter_mut()
                    .for_each(|spec| spec.visible = false),
                2 => layout.columns[0].width = f32::NAN,
                3 => layout.columns[0].width = 1025.0,
                4 => {
                    layout.secondary = Some(Sort {
                        column: Column::Title,
                        descending: false,
                    })
                }
                5 => {
                    layout.primary = Some(Sort {
                        column: Column::Title,
                        descending: false,
                    });
                    layout.secondary = layout.primary;
                }
                _ => invalid.layouts[1].name = "cOmPaCt".into(),
            }
            assert!(invalid.validate().is_err(), "edit {edit}");
        }
    }
    #[test]
    fn version_twelve_migrates_without_new_fields_and_refuses_smuggled_layouts() {
        let current = Preferences::defaults(Path::new("/tmp"));
        let mut old = serde_json::to_value(&current).unwrap();
        old["version"] = 12.into();
        for profile in old["profiles"].as_object_mut().unwrap().values_mut() {
            profile.as_object_mut().unwrap().remove("library_layout");profile.as_object_mut().unwrap().remove("waveforms");
        }
        let (migrated, changed) = storage::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(changed);
        assert_eq!(migrated, current);
        for field in [
            serde_json::Value::Null,
            serde_json::to_value(Config::default()).unwrap(),
        ] {
            old["profiles"]["Studio"]["library_layout"] = field;
            assert!(storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
        }
        old["version"] = 15.into();
        assert!(storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    }
    #[test]
    fn explicit_atomic_save_retains_column_order_width_density_and_both_sorts() {
        let root =
            std::env::temp_dir().join(format!("omatainer-layout-storage-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        struct Files(std::path::PathBuf);
        impl Drop for Files {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let files = Files(root);
        let path = files.0.join("preferences.json");
        let mut current = Preferences::defaults(&files.0);
        let config = &mut current.profiles.get_mut("Studio").unwrap().library_layout;
        config.active = "Artwork".into();
        let layout = config.current_mut();
        layout.columns.swap(0, 5);
        layout.columns[0].width = 320.0;
        layout.primary = Some(Sort {
            column: Column::Artist,
            descending: true,
        });
        layout.secondary = Some(Sort {
            column: Column::Title,
            descending: false,
        });
        let cancel = AtomicBool::new(false);
        storage::save(&path, &current, storage::Overwrite::New, &cancel).unwrap();
        let loaded = storage::load(&path, &cancel).unwrap();
        assert!(!loaded.migrated);
        assert_eq!(loaded.preferences, current);
        let before = std::fs::read(&path).unwrap();
        assert!(storage::save(&path, &current, storage::Overwrite::Exact(None), &cancel).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
