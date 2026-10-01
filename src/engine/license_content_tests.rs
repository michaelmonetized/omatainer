use super::*;
use std::collections::BTreeSet;

#[test]
fn manifest_members_match_the_actual_generated_audio_and_seeded_patterns() {
    let catalog =
        crate::licenses::Catalog::parse(crate::licenses::MANIFEST, crate::licenses::NOTICES)
            .unwrap();
    let members = |id: &str| -> BTreeSet<String> {
        catalog
            .manifest
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap()
            .members
            .iter()
            .cloned()
            .collect()
    };
    let kit = build_kit(8000);
    assert_eq!(
        members("factory:drum-kit"),
        kit.iter().map(|s| s.path.clone()).collect()
    );
    let banks = build_pad_banks(8000);
    assert_eq!(
        members("factory:pad-banks"),
        banks.iter().flatten().map(|s| s.path.clone()).collect()
    );
    let (drums, harmony) = demo_stems(8000, 120.0);
    assert_eq!(
        members("factory:session-stems"),
        [drums.path.clone(), harmony.path.clone()]
            .into_iter()
            .collect()
    );
    let (_engine, rt) = Engine::headless_for_test(8000, 64);
    assert_eq!(
        members("factory:session-patterns"),
        rt.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|clip| clip.kind != ClipKind::Empty)
            .map(|clip| clip.name.clone())
            .collect()
    );
}
