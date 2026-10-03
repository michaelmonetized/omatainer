use super::*;
#[test]
fn translated_templates_keep_values_and_thread_local_frames_independent() {
    let _locale = scope(Locale::Spanish);
    assert_eq!(text("Project"), "Proyecto");
    assert_eq!(format("Actions for {visible}", &["Café / 東京 / مشروع".into()]), "Acciones para Café / 東京 / مشروع");
    assert_eq!(number(1.25,2), "1,25");
    assert_eq!(parse_number("1,25"),Some(1.25));
    assert_eq!(parse_number("1.25"),Some(1.25));
    assert_eq!(parse_number("NaN"),None);
    assert_eq!(timestamp(0), "01/01/1970 00:00:00 UTC");
    { let _locale = scope(Locale::German); assert_eq!(text("Project"),"Projekt"); assert_eq!(timestamp(0),"01.01.1970 00:00:00 UTC"); }
    assert_eq!(text("Project"),"Proyecto");
    assert_eq!(std::thread::spawn(|| text("Project")).join().unwrap(),"Project");
}
#[test]
fn canonical_full_casefold_search_preserves_original_codepoints() {
    let original="Cafe\u{301} / Straße / Σίσυφος / 東京 / العربية";
    let saved=original.to_owned();
    assert!(search_key(original).contains(&search_key("CAFÉ")));
    assert!(search_key(original).contains(&search_key("STRASSE")));
    assert_eq!(search_key("σςΣ"),"σσσ");
    assert_eq!(original,saved);
    assert_ne!(original.as_bytes(),search_key(original).as_bytes());
}
#[test]
fn shipped_catalogues_have_complete_reorderable_placeholder_sets() {
    fn fields(template: &str) -> Vec<usize> {
        let mut out=Vec::new();let mut chars=template.chars().peekable();
        while let Some(c)=chars.next() {
            if (c=='{' && chars.peek()==Some(&'{')) || (c=='}' && chars.peek()==Some(&'}')) { chars.next();continue; }
            if c=='{' { let mut id=String::new();loop {let c=chars.next().expect("closed field");if c=='}' {break;}id.push(c);}out.push(id.parse().expect("numeric field")); }
        }
        out.sort_unstable();out
    }
    for catalogue in [&*SPANISH,&*GERMAN] {
        for (key,value) in catalogue {
            assert_eq!(fields(&ENGLISH[key]),fields(value),"{key}");
        }
    }
}
