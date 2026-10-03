//! Portable bindings contain no device names, paths or other profile settings.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bundle {
    version: u32,
    enabled: bool,
    #[serde(deserialize_with = "unique_map")]
    bindings: BTreeMap<String, Option<Shortcut>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_bindings_validate_before_replacing_only_shortcuts() {
        let mut profile = Profile::defaults(std::path::Path::new("/private-library"));
        profile.shortcuts.insert("play_a".into(), Some(Shortcut {key:"G".into(),ctrl:false,shift:false,alt:false}));
        let bundle = Bundle::from_profile(&profile);
        let bytes = serde_json::to_vec(&bundle).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private-library"));
        let mut target = Profile::defaults(std::path::Path::new("/other-library"));
        target.library_roots.push("unrelated-relative-draft".into());
        assert!(target.validate().is_err());
        let mut expected = target.clone(); expected.shortcuts = profile.shortcuts.clone();
        Bundle::decode(&bytes).unwrap().apply(&mut target).unwrap();
        assert_eq!(target, expected);
        let original = target.clone();
        for invalid in [
            br#"{"version":99,"enabled":true,"bindings":{}}"#.as_slice(),
            br#"{"version":1,"enabled":true,"bindings":{"transport":{"key":"Q","ctrl":false,"shift":false,"alt":false}}}"#.as_slice(),
            br#"{"version":1,"enabled":true,"bindings":{"play_a":null,"play_a":null}}"#.as_slice(),
            br#"{"version":1,"enabled":true,"bindings":{},"paths":[]}"#.as_slice(),
        ] {
            assert!(Bundle::decode(invalid).is_err());
            assert_eq!(target, original);
        }
    }
}

impl Bundle {
    /// Capture this profile's binding overrides and performance shortcut switch.
    /// Takes a profile; returns a portable versioned binding document.
    pub(crate) fn from_profile(profile: &Profile) -> Self {
        Self { version: 1, enabled: profile.shortcuts_enabled, bindings: profile.shortcuts.clone() }
    }

    /// Validate a binding document before changing its destination profile.
    /// Takes the target profile; returns success after replacing only validated bindings.
    pub(crate) fn apply(&self, profile: &mut Profile) -> Result<(), String> {
        self.validate()?;
        profile.shortcuts_enabled = self.enabled;
        profile.shortcuts = self.bindings.clone();
        Ok(())
    }

    /// Validate only the shortcut fields carried by this portable document.
    /// Takes this bundle; returns success or refusal independently of unrelated draft settings.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != 1 { return Err("Unsupported shortcut export version; bindings were preserved".into()); }
        let mut candidate = Profile::defaults(std::path::Path::new("/tmp"));
        candidate.shortcuts_enabled = self.enabled;
        candidate.shortcuts = self.bindings.clone();
        candidate.validate()
    }

    /// Decode and validate bounded, strict binding JSON without applying it.
    /// Takes original file bytes; returns a validated bundle or a refusal.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 32_768 { return Err("Shortcut export exceeds 32 KiB".into()); }
        let bundle: Self = serde_json::from_slice(bytes).map_err(|error| format!("Invalid shortcut export: {error}"))?;
        bundle.validate()?;
        Ok(bundle)
    }
}
