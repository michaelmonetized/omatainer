//! Named panel layouts stay independent of musical and device settings.
use super::{unique_map, BTreeMap, Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Panel {
    Decks,
    Sampler,
    Library,
    Session,
}
impl Panel {
    pub const ALL: [Self; 4] = [Self::Decks, Self::Sampler, Self::Library, Self::Session];
    /// Name a persistent panel without changing its identity.
    /// Takes the panel; returns its readable catalogue label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Decks => "Decks",
            Self::Sampler => "Sampler",
            Self::Library => "Library",
            Self::Session => "Session and mixer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub panel: Panel,
    pub visible: bool,
    pub height: f32,
    pub detached: bool,
    pub window_size: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Layout {
    pub panels: Vec<Entry>,
}
impl Layout {
    /// Check complete panel identity and finite, usable window dimensions.
    /// Takes the layout; returns an error before any invalid layout is applied.
    pub fn validate(&self) -> Result<(), String> {
        if self.panels.len() != Panel::ALL.len()
            || Panel::ALL.iter().any(|panel| {
                self.panels
                    .iter()
                    .filter(|entry| entry.panel == *panel)
                    .count()
                    != 1
            })
        {
            return Err("A workspace needs each panel exactly once".into());
        }
        if !self.panels.iter().any(|entry| entry.visible) {
            return Err("Keep at least one workspace panel visible".into());
        }
        for entry in &self.panels {
            if !entry.height.is_finite()
                || entry.height != 0.0 && !(64.0..=2048.0).contains(&entry.height)
            {
                return Err("Panel height must be automatic or between 64 and 2048 points".into());
            }
            if entry
                .window_size
                .iter()
                .any(|size| !size.is_finite() || !(240.0..=4096.0).contains(size))
            {
                return Err("Window dimensions must be between 240 and 4096 points".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub active: String,
    #[serde(deserialize_with = "unique_map")]
    pub saved: BTreeMap<String, Layout>,
}
impl Default for Config {
    fn default() -> Self {
        let production = Layout {
            panels: Panel::ALL
                .into_iter()
                .map(|panel| Entry {
                    panel,
                    visible: true,
                    height: 0.0,
                    detached: false,
                    window_size: [960.0, 720.0],
                })
                .collect(),
        };
        let mut mix = production.clone();
        mix.panels.rotate_right(1);
        for entry in &mut mix.panels {
            entry.visible = entry.panel == Panel::Session;
        }
        let mut dj = production.clone();
        for entry in &mut dj.panels {
            entry.visible = entry.panel != Panel::Session;
        }
        Self {
            active: "Production".into(),
            saved: BTreeMap::from([
                ("Production".into(), production),
                ("Mix".into(), mix),
                ("DJ".into(), dj),
            ]),
        }
    }
}
impl Config {
    /// Read the selected named layout without assuming imported data is valid.
    /// Takes this configuration; returns the active layout when it exists.
    pub fn current(&self) -> Option<&Layout> {
        self.saved.get(&self.active)
    }

    /// Validate names, the selected layout and every saved panel configuration.
    /// Takes this configuration; returns a readable error without modifying it.
    pub fn validate(&self) -> Result<(), String> {
        if self.saved.is_empty() || self.saved.len() > 32 {
            return Err("Keep between 1 and 32 workspaces".into());
        }
        if self.current().is_none() {
            return Err("The active workspace does not exist".into());
        }
        for (name, layout) in &self.saved {
            validate_name(name)?;
            layout
                .validate()
                .map_err(|error| format!("{name}: {error}"))?;
        }
        Ok(())
    }

    /// Copy the current layout under an unused name and select it.
    /// Takes the exact new name; returns an error without changing saved layouts on refusal.
    pub fn copy(&mut self, name: &str) -> Result<(), String> {
        validate_name(name)?;
        if self.saved.len() == 32 {
            return Err("Keep no more than 32 workspaces".into());
        }
        if self.saved.contains_key(name) {
            return Err("That workspace name already exists".into());
        }
        let layout = self
            .current()
            .ok_or("The active workspace does not exist")?
            .clone();
        layout.validate()?;
        self.saved.insert(name.into(), layout);
        self.active = name.into();
        Ok(())
    }

    /// Remove the selected layout while retaining a selectable workspace.
    /// Takes no arguments; returns an error if removing it would leave no layout.
    pub fn remove_current(&mut self) -> Result<(), String> {
        if self.saved.len() < 2 || self.current().is_none() {
            return Err("Keep at least one workspace".into());
        }
        self.saved.remove(&self.active);
        self.active = self.saved.keys().next().unwrap().clone();
        Ok(())
    }
}

/// Check a workspace name while preserving its exact Unicode spelling.
/// Takes the proposed name; returns an error for empty, excessive or invisible names.
fn validate_name(name: &str) -> Result<(), String> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > 80
        || name.chars().any(char::is_control)
    {
        return Err(
            "Workspace names need 1–80 visible characters without surrounding spaces".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_layout_copy_reorder_hide_resize_and_remove_preserve_exact_values() {
        let mut config = Config::default();
        config.validate().unwrap();
        config.copy("Mix 東京 Café").unwrap();
        let layout = config.saved.get_mut(&config.active).unwrap();
        layout.panels.swap(0, 3);
        layout.panels[0].height = 320.0;
        layout.panels[0].detached = true;
        layout.panels[0].window_size = [1200.0, 600.0];
        layout.panels[1].visible = false;
        config.validate().unwrap();
        let bytes = serde_json::to_vec(&config).unwrap();
        assert_eq!(serde_json::from_slice::<Config>(&bytes).unwrap(), config);
        let before = config.clone();
        assert!(config.copy("Mix 東京 Café").is_err());
        assert_eq!(config, before);
        assert!(config.copy(" invalid ").is_err());
        assert_eq!(config, before);
        config.remove_current().unwrap();
        assert!(!config.saved.contains_key("Mix 東京 Café"));
        config.validate().unwrap();
    }
    #[test]
    fn forged_workspace_structure_sizes_and_duplicate_names_are_refused() {
        let config = Config::default();
        for value in [f32::NAN, f32::INFINITY, -1.0, 63.0, 2049.0] {
            let mut bad = config.clone();
            bad.saved.get_mut("Production").unwrap().panels[0].height = value;
            assert!(bad.validate().is_err());
        }
        for value in [0.0, 239.0, 4097.0, f32::NAN] {
            let mut bad = config.clone();
            bad.saved.get_mut("Production").unwrap().panels[0].window_size[0] = value;
            assert!(bad.validate().is_err());
        }
        let mut bad = config.clone();
        bad.active = "Missing".into();
        assert!(bad.validate().is_err());
        let mut bad = config.clone();
        bad.saved.get_mut("Production").unwrap().panels[0].panel = Panel::Sampler;
        assert!(bad.validate().is_err());
        let layout = serde_json::to_string(config.current().unwrap()).unwrap();
        let duplicate =
            format!("{{\"active\":\"Same\",\"saved\":{{\"Same\":{layout},\"Same\":{layout}}}}}");
        assert!(serde_json::from_str::<Config>(&duplicate).is_err());
        let mut unknown = serde_json::to_value(&config).unwrap();
        unknown["positions"] = "unrecognized".into();
        assert!(serde_json::from_value::<Config>(unknown).is_err());
    }
}
