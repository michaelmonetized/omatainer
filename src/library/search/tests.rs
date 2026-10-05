use super::*;
fn row(fields: &Annotations) -> Row<'_> {
    Row {
        title: "Café at midnight",
        artist: "Nine Inch Nails",
        key: "Am",
        bpm: Some(124.0),
        seconds: Some(210.0),
        played: false,
        annotations: fields,
    }
}
#[test]
fn quoted_fields_ranges_unicode_and_annotations_combine_against_current_metadata() {
    let fields = Annotations {
        rating: 4,
        color: Some([255, 102, 0]),
        group: "Peak hour".into(),
        tags: vec!["clean edit".into()],
        notes: "Dinner request".into(),
    };
    assert!(Query::parse("title:Cafe\u{301} artist:\"Nine Inch Nails\" bpm:120..128 length:3:00..4:00 rating>=4 tag:\"clean edit\" key:Am played:no color:#FF6600 group:Peak note:Dinner").unwrap().matches(row(&fields)));
    for query in [
        "title:Nails",
        "artist:midnight",
        "bpm>=125",
        "length<=200",
        "played:yes",
        "key:C",
        "rating:5",
        "tag:clean",
    ] {
        assert!(
            !Query::parse(query).unwrap().matches(row(&fields)),
            "{query}"
        );
    }
    assert!(Query::parse("Café Nails").unwrap().matches(row(&fields)));
}
#[test]
fn malformed_reserved_queries_fail_visibly_and_unknown_numeric_values_do_not_match() {
    for query in [
        "bpm:NaN",
        "bpm:128..120",
        "bpm>120",
        "length:3:99",
        "rating:6",
        "rating>=4.5",
        "played:maybe",
        "artist:\"unterminated",
        "color:red",
        "color:#ééé",
        "tag:",
        "unknown:field",
    ] {
        assert!(Query::parse(query).is_err(), "{query}");
    }
    let fields = Annotations::default();
    let mut empty = row(&fields);
    empty.bpm = None;
    empty.seconds = None;
    assert!(!Query::parse("bpm>=0 length>=0").unwrap().matches(empty));
    assert!(Query::parse(&"x".repeat(4097)).is_err());
    assert!(Query::parse(&"x ".repeat(65)).is_err());
}
