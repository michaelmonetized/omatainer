//! Authenticated declarative controller data with compiled protocol capabilities.
use super::{Action, Binding, MidiMap, MsgKind, UnmappedNotes};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
pub(crate) mod identity;
pub(crate) mod runtime;
mod storage;
#[cfg(test)]
mod tests;
pub(crate) const SCHEMA: u32 = 1;
pub(crate) const MAX_PROFILE: usize = 65536;
const MAX_CATALOG: usize = 256 * 1024;
const SIGNATURE_DOMAIN: &[u8] = b"Omatainer controller catalog schema 1\n";
const SIGNER: &str = "ed25519-v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Driver {
    Generic,
    Ns7,
    Apc40Mk2,
    Sp1,
    Mpd232,
}
impl Driver {
    pub fn name(self) -> &'static str {
        match self {
            Self::Generic => "Class-compliant MIDI",
            Self::Ns7 => "Numark NS7 (original)",
            Self::Apc40Mk2 => "Akai APC40 mkII",
            Self::Sp1 => "Pioneer DDJ-SP1",
            Self::Mpd232 => "Akai MPD232 (LiveLite)",
        }
    }
    fn capability(self) -> &'static str {
        match self {
            Self::Generic => "midi1-binding-v6",
            Self::Ns7 => "ns7-original-v1",
            Self::Apc40Mk2 => "apc40-mkii-v1",
            Self::Sp1 => "ddj-sp1-v1",
            Self::Mpd232 => "mpd232-v1",
        }
    }
    fn factory(self) -> Result<MidiMap, String> {
        match self {
            Self::Generic => Ok(super::class_compliant()),
            Self::Ns7 => Ok(super::surface::numark_ns7()),
            Self::Apc40Mk2 => Ok(super::akai_apc40_mk2()),
            Self::Sp1 => Ok(super::surface::pioneer_sp1()),
            Self::Mpd232 => super::surface::mpd232::parse(include_bytes!(
                "../../../tests/fixtures/mpd232-livelite.syx"
            ))
            .map_err(|e| e.to_string()),
        }
    }
    fn usb(self) -> Option<(u16, u16)> {
        match self {
            Self::Generic => None,
            Self::Ns7 => Some((0x15e4, 0x0071)),
            Self::Apc40Mk2 => Some((0x09e8, 0x0029)),
            Self::Sp1 => Some((0x08e4, 0x0181)),
            Self::Mpd232 => Some((0x09e8, 0x0036)),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Usb {
    pub vendor: u16,
    pub product: u16,
    pub minimum_release: Option<u16>,
    pub maximum_release: Option<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ports {
    pub input: u8,
    pub output: u8,
    pub role: String,
    pub other_roles: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Initialization {
    None,
    Inquiry,
    Apc40Mk2Host41,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fixture {
    pub bytes: [u8; 3],
    pub action: Action,
    pub deck: u8,
    pub extra: u16,
    pub evidence: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    pub license: String,
    pub authored_by: String,
    pub manufacturer_documents: Vec<String>,
    pub licensed_sources: Vec<String>,
    pub synthetic_fixture_claim: String,
    pub physical_receipts: Vec<String>,
    pub current_physical_qualification: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    pub schema: u32,
    pub preset_version: u32,
    pub id: String,
    pub version: String,
    pub model: String,
    pub variant: String,
    pub minimum_app: [u16; 3],
    pub driver: Driver,
    pub required_capabilities: Vec<String>,
    pub usb: Vec<Usb>,
    pub protocol: String,
    pub firmware_constraint: String,
    pub ports: Ports,
    pub initialization: Initialization,
    pub feedback: Driver,
    pub layer: String,
    pub unmapped_notes_live: bool,
    pub bindings: Vec<Binding>,
    pub fixtures: Vec<Fixture>,
    pub provenance: Provenance,
}
fn visible(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}
fn version(value: &str) -> Result<[u16; 3], String> {
    let parts = value
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Profile version needs three bounded decimal numbers")?;
    parts
        .try_into()
        .map_err(|_| "Profile version needs three decimal numbers".into())
}
fn hex(value: &str, bytes: usize) -> Result<Vec<u8>, String> {
    if value.len() != bytes * 2 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid authenticated controller hexadecimal field".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Profile {
    /// Validate one complete declarative profile.
    /// Takes owned data; returns a full refusal for incompatible schema, capabilities, identity, bindings or fixture results, never a partial mapping.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA
            || self.preset_version != super::presets::VERSION
            || self.minimum_app > [0, 1, 0]
            || self.layer != "factory_overlay"
            || !visible(&self.id, 80)
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !visible(&self.model, 128)
            || !visible(&self.variant, 128)
            || !visible(&self.protocol, 256)
            || !visible(&self.firmware_constraint, 512)
            || self.bindings.len() > super::learn::MAX_MAPPINGS
            || self.usb.is_empty()
            || self.usb.len() > 8
            || self.ports.input > 15
            || self.ports.output != self.ports.input
            || !visible(&self.ports.role, 128)
            || self.ports.other_roles.len() > 15
            || self.ports.other_roles.iter().any(|s| !visible(s, 128))
        {
            return Err(
                "Controller profile exceeds its schema, app, label, port or binding bounds".into(),
            );
        }
        version(&self.version)?;
        let mut ids = BTreeSet::new();
        for usb in &self.usb {
            if usb.vendor == 0
                || usb.product == 0
                || !ids.insert((usb.vendor, usb.product))
                || usb
                    .minimum_release
                    .zip(usb.maximum_release)
                    .is_some_and(|(a, b)| a > b)
                || self
                    .driver
                    .usb()
                    .is_some_and(|id| id != (usb.vendor, usb.product))
            {
                return Err(
                    "Controller USB identity or compiled decoder model is incompatible".into(),
                );
            }
        }
        let expected: Vec<_> = BTreeSet::from([
            "midi1-binding-v6".to_owned(),
            self.driver.capability().to_owned(),
        ])
        .into_iter()
        .collect();
        let requested: BTreeSet<_> = self.required_capabilities.iter().cloned().collect();
        if requested.into_iter().collect::<Vec<_>>() != expected
            || self.required_capabilities.len() != expected.len()
        {
            return Err("Controller requires unavailable compiled capabilities".into());
        }
        if self.feedback != self.driver {
            return Err("Controller feedback requires the selected compiled driver".into());
        }
        if self.initialization == Initialization::Apc40Mk2Host41 && self.driver != Driver::Apc40Mk2
            || self.driver == Driver::Apc40Mk2
                && self.initialization != Initialization::Apc40Mk2Host41
            || self.driver == Driver::Generic && self.initialization != Initialization::None
        {
            return Err(
                "Controller initialization is incompatible with its compiled protocol".into(),
            );
        }
        if self.provenance.license != "MIT"
            || !visible(&self.provenance.authored_by, 128)
            || !visible(&self.provenance.synthetic_fixture_claim, 512)
            || self.provenance.manufacturer_documents.len() > 16
            || self.provenance.licensed_sources.len() > 16
            || self.provenance.physical_receipts.len() > 16
            || self.provenance.current_physical_qualification
        {
            return Err("Controller data cannot establish current physical qualification".into());
        }
        for reference in self
            .provenance
            .manufacturer_documents
            .iter()
            .chain(&self.provenance.licensed_sources)
            .chain(&self.provenance.physical_receipts)
        {
            if !visible(reference, 2048) || !reference.starts_with("https://") {
                return Err("Invalid controller provenance reference".into());
            }
        }
        if self.driver != Driver::Generic {
            let factory = self.driver.factory()?;
            if self.bindings != factory.bindings
                || self.unmapped_notes_live != (factory.unmapped_notes == UnmappedNotes::Live)
                || self.ports.input != 0
                || self.protocol != self.driver.capability()
            {
                return Err("Profile data conflicts with fixed compiled protocol addresses; use learned overrides or a new reviewed app capability".into());
            }
        }
        self.map().validate().map_err(|e| e.to_string())?;
        if self.fixtures.is_empty() || self.fixtures.len() > 32 {
            return Err(
                "Controller profile needs 1–32 independent recorded or synthetic binding checks"
                    .into(),
            );
        }
        for fixture in &self.fixtures {
            if !visible(&fixture.evidence, 256)
                || fixture.bytes[1..].iter().any(|b| *b >= 128)
                || !self.bindings.iter().any(|b| {
                    let class = match b.kind {
                        MsgKind::Note => 0x90,
                        MsgKind::Cc | MsgKind::Cc14 | MsgKind::CcRel => 0xb0,
                        MsgKind::Pitch => 0xe0,
                    };
                    fixture.bytes[0] & 0xf0 == class
                        && (b.ch == 0xff || b.ch == fixture.bytes[0] & 15)
                        && (b.kind == MsgKind::Pitch || b.data == fixture.bytes[1])
                        && b.action == fixture.action
                        && b.deck == fixture.deck
                        && b.extra == fixture.extra
                })
            {
                return Err(
                    "Controller message fixture does not decode to its declared binding".into(),
                );
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_PROFILE {
            return Err("Controller profile exceeds 64 KiB".into());
        }
        Ok(())
    }
    /// Decode strict bounded controller data.
    /// Takes JSON bytes; returns the completely validated profile without opening a device or executing code.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PROFILE {
            return Err("Controller profile exceeds 64 KiB".into());
        }
        let profile: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        profile.validate()?;
        Ok(profile)
    }
    /// Build a factory layer with a reviewed compiled decoder name.
    /// Takes this validated profile; returns bindings without changing learned overrides or selecting executable code by a remote display name.
    pub fn map(&self) -> MidiMap {
        MidiMap {
            name: self.driver.name().into(),
            matchers: vec![],
            bindings: self.bindings.clone(),
            unmapped_notes: if self.unmapped_notes_live {
                UnmappedNotes::Live
            } else {
                UnmappedNotes::Ignore
            },
        }
    }
    pub fn matches(&self, device: &identity::Device) -> bool {
        device.port == self.ports.input
            && self.usb.iter().any(|usb| {
                usb.vendor == device.vendor
                    && usb.product == device.product
                    && usb.minimum_release.is_none_or(|v| device.release >= v)
                    && usb.maximum_release.is_none_or(|v| device.release <= v)
            })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: String,
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub schema: u32,
    pub generation: u64,
    pub version: String,
    pub release: String,
    pub profiles: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: u32,
    signer: String,
    payload: String,
    signature: String,
}
impl Catalog {
    /// Authenticate metadata before parsing any profile pointer.
    /// Takes bounded signed bytes and the highest accepted generation; returns a pinned catalog or rejects tampering, replay and incompatible release paths.
    pub fn decode(bytes: &[u8], minimum: u64) -> Result<Self, String> {
        if bytes.len() > MAX_CATALOG {
            return Err("Controller catalog exceeds 256 KiB".into());
        }
        let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if envelope.schema != SCHEMA
            || envelope.signer != SIGNER
            || envelope.payload.len() > MAX_CATALOG
        {
            return Err("Unknown controller catalog signer or schema".into());
        }
        let key = hex(
            include_str!("../../../profiles/trust/ed25519-v1.pub").trim(),
            32,
        )?;
        let signature = hex(&envelope.signature, 64)?;
        let mut message = SIGNATURE_DOMAIN.to_vec();
        message.extend_from_slice(envelope.payload.as_bytes());
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
            .verify(&message, &signature)
            .map_err(|_| "Controller catalog signature was rejected")?;
        let catalog: Self = serde_json::from_str(&envelope.payload).map_err(|e| e.to_string())?;
        catalog.validate(minimum)?;
        Ok(catalog)
    }
    fn validate(&self, minimum: u64) -> Result<(), String> {
        if self.schema != SCHEMA
            || self.generation == 0
            || self.generation < minimum
            || self.profiles.is_empty()
            || self.profiles.len() > 64
            || self.release != format!("controller-profiles-{}", self.version)
        {
            return Err(
                "Controller catalog schema, generation or immutable release is incompatible".into(),
            );
        }
        version(&self.version)?;
        let mut ids = BTreeSet::new();
        let mut files = BTreeSet::new();
        for entry in &self.profiles {
            version(&entry.version)?;
            if !visible(&entry.id, 80)
                || !entry
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || entry.file != format!("{}-{}.json", entry.id, entry.version)
                || !ids.insert(&entry.id)
                || !files.insert(&entry.file)
                || entry.bytes == 0
                || entry.bytes > MAX_PROFILE
                || hex(&entry.sha256, 32).is_err()
            {
                return Err("Invalid or repeated immutable controller profile entry".into());
            }
        }
        Ok(())
    }
    /// Check the exact acquired bytes and data identity.
    /// Takes an authenticated entry and downloaded/cache bytes; returns the validated profile only when length, hash, ID and version all match.
    pub fn profile(&self, entry: &Entry, bytes: &[u8]) -> Result<Profile, String> {
        if !self.profiles.iter().any(|e| {
            e.id == entry.id
                && e.sha256 == entry.sha256
                && e.file == entry.file
                && e.bytes == entry.bytes
                && e.version == entry.version
        }) || bytes.len() != entry.bytes
            || hash(bytes) != entry.sha256
        {
            return Err("Controller profile hash, size or catalog membership changed".into());
        }
        let profile = Profile::decode(bytes)?;
        if profile.id != entry.id || profile.version != entry.version {
            return Err("Controller profile identity disagrees with authenticated catalog".into());
        }
        Ok(profile)
    }
}
/// Produce the four owned declarative factory profiles.
/// Takes no installed script or vendor code; returns original data paired with existing reviewed compiled protocol handlers, with current physical qualification pending.
pub(crate) fn bundled() -> Result<Vec<Profile>, String> {
    let mappings = [
        (
            Driver::Ns7,
            super::surface::numark_ns7(),
            "numark-ns7-original",
        ),
        (Driver::Apc40Mk2, super::akai_apc40_mk2(), "akai-apc40-mkii"),
        (
            Driver::Sp1,
            super::surface::pioneer_sp1(),
            "pioneer-ddj-sp1",
        ),
        (
            Driver::Mpd232,
            super::surface::mpd232::parse(include_bytes!(
                "../../../tests/fixtures/mpd232-livelite.syx"
            ))
            .map_err(|e| e.to_string())?,
            "akai-mpd232-livelite",
        ),
    ];
    mappings.into_iter().map(|(driver,map,id)|{
        let (vendor,product)=driver.usb().unwrap();
        let fixture=match driver{Driver::Ns7=>Fixture{bytes:[0x90,0x11,127],action:Action::DeckPlay,deck:0,extra:0,evidence:"Original native NS7 protocol fixture; physical result is separate".into()},Driver::Apc40Mk2=>Fixture{bytes:[0x90,32,127],action:Action::Clip,deck:0,extra:0,evidence:"Manufacturer v1.2 top-left grid address, original fixture".into()},Driver::Sp1=>Fixture{bytes:[0x90,0x58,127],action:Action::DeckSync,deck:0,extra:0,evidence:"Manufacturer Sync address, original fixture".into()},_=>Fixture{bytes:[0xb0,12,77],action:Action::TrackFader,deck:0,extra:0,evidence:"Owned LiveLite preset and captured bank-A CC12 address".into()}};
        let documents=match driver{Driver::Apc40Mk2=>vec!["https://cdn.inmusicbrands.com/akai/attachments/apc40II/APC40Mk2_Communications_Protocol_v1.2.pdf".into()],Driver::Sp1=>vec!["https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-SP1/DDJ-SP1_List_of_MIDI_Messages_E.pdf".into()],Driver::Mpd232=>vec!["https://cdn.inmusicbrands.com/akai/attachments/MPD232/MPD232-User_Guide-v1.1.pdf".into()],_=>vec![]};
        let required_capabilities=BTreeSet::from(["midi1-binding-v6".into(),driver.capability().into()]).into_iter().collect();
        let profile=Profile{schema:SCHEMA,preset_version:super::presets::VERSION,id:id.into(),version:"1.0.0".into(),model:driver.name().into(),variant:if driver==Driver::Mpd232{"User-owned LiveLite preset".into()}else{"Exact USB model; other generations require another profile".into()},minimum_app:[0,1,0],driver,required_capabilities,usb:vec![Usb{vendor,product,minimum_release:None,maximum_release:None}],protocol:driver.capability().into(),firmware_constraint:"No firmware-wide claim; record USB release and actual inquiry response during qualification".into(),ports:Ports{input:0,output:0,role:"performance".into(),other_roles:if driver==Driver::Mpd232{vec!["Port B: explicit sequencer role".into(),"DIN A: explicit external role".into(),"DIN B: explicit external role".into()]}else{vec![]}},feedback:driver,initialization:match driver{Driver::Apc40Mk2=>Initialization::Apc40Mk2Host41,Driver::Mpd232=>Initialization::Inquiry,_=>Initialization::None},layer:"factory_overlay".into(),unmapped_notes_live:map.unmapped_notes==UnmappedNotes::Live,bindings:map.bindings,fixtures:vec![fixture],provenance:Provenance{license:"MIT".into(),authored_by:"Omatainer original native mappings".into(),manufacturer_documents:documents,licensed_sources:vec!["https://github.com/michaelmonetized/omatainer/blob/stack/app-completion/LICENSE".into()],synthetic_fixture_claim:"Schema and message decoding only; does not promote current physical qualification".into(),physical_receipts:vec!["https://github.com/michaelmonetized/omatainer/blob/a4020338a805ce2efd690da34f898892806d6e32/docs/validation/powered-hub-controller-qualification.md".into()],current_physical_qualification:false}};
        profile.validate()?;Ok(profile)
    }).collect()
}
/// Export versioned original profile data for independently signed distribution.
/// Takes a new output directory; writes four validated bounded files and refuses an existing destination rather than overwriting release data.
pub(crate) fn export(directory: &Path) -> Result<(), String> {
    if directory.exists() {
        return Err("Controller profile export requires a new directory".into());
    }
    let profiles = bundled()?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    for profile in profiles {
        let path = directory.join(format!("{}-{}.json", profile.id, profile.version));
        let bytes = serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_PROFILE {
            return Err("Formatted profile exceeds 64 KiB".into());
        }
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Inspect native MIDI inventories without changing hardware.
/// Takes no arguments; returns exact observed input/output USB identities and matching profile IDs, with all physical checks pending.
pub(crate) fn inspect() -> Result<serde_json::Value, String> {
    let input = midir::MidiInput::new("omatainer-profile-inspect").map_err(|e| e.to_string())?;
    let output = midir::MidiOutput::new("omatainer-profile-inspect").map_err(|e| e.to_string())?;
    let profiles = bundled()?;
    let inputs=input.ports().iter().take(256).map(|p|{let id=p.id();let device=identity::Device::discover(&id);let matches=device.as_ref().map_or(Vec::new(),|d|profiles.iter().filter(|p|p.matches(d)).map(|p|p.id.clone()).collect());Ok(serde_json::json!({"id":id,"name":input.port_name(p).map_err(|e|e.to_string())?,"usb":device,"profile_candidates":matches,"input_open":false,"physical_qualification":"pending"}))}).collect::<Result<Vec<_>,String>>()?;
    let outputs=output.ports().iter().take(256).map(|p|{let id=p.id();Ok(serde_json::json!({"id":id,"name":output.port_name(p).map_err(|e|e.to_string())?,"usb":identity::Device::discover(&id),"output_open":false}))}).collect::<Result<Vec<_>,String>>()?;
    Ok(
        serde_json::json!({"schema":SCHEMA,"platform":format!("{}-{}",std::env::consts::ARCH,std::env::consts::OS),"inputs":inputs,"outputs":outputs}),
    )
}
