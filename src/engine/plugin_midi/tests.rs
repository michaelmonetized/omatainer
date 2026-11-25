use super::*;
use crate::engine::test_alloc;

#[test]
fn fixed_plugin_midi_storage_initializes_on_a_small_worker_stack_and_never_grows_in_callbacks() {
    std::thread::Builder::new()
        .name("plugin-midi-storage".into())
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut routing = Routing::default();
            assert_eq!(routing.frames.len(), MAX_TRACKS);
            assert_eq!(routing.clips.len(), MAX_TRACKS);
            assert_eq!(
                test_alloc::measure(|| {
                    for track in 0..MAX_TRACKS {
                        for event in 0..EVENTS {
                            assert!(routing.push(track, [0x90, (event % 128) as u8, 100]));
                        }
                        assert_eq!(routing.events(track).len(), EVENTS);
                        assert_eq!(routing.events(track)[EVENTS - 1], [0x90, 127, 100]);
                        assert!(!routing.push(track, [0x80, 127, 0]));
                        assert!(routing.refused(track));
                    }
                    routing.reset();
                    for track in 0..MAX_TRACKS {
                        assert!(!routing.refused(track));
                        assert!(routing.events(track).is_empty());
                        assert!(routing.push(track, [0x90, 60, 90]));
                    }
                    routing.next();
                    assert!(routing.lengths.iter().all(|length| *length == 0));
                }),
                test_alloc::Counts::default()
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn mpe_plugins_keep_sources_sustain_and_reused_channels_independent_without_allocating() {
    use crate::engine::mpe::{State, Values};
    use crate::plugin_host::expression::Event;
    let mut routing = Routing::default();
    let mut first = State::default();
    for bytes in [[0xb0,101,0],[0xb0,100,6],[0xb0,6,4]] { first.input(&bytes); }
    let mut second = first;
    let one = InputKey::Midi { source: 41, ch: 1, note: 60 };
    let two = InputKey::Midi { source: 42, ch: 1, note: 60 };
    let counts = test_alloc::measure(|| {
        routing.expression_on(ExpressionOwner::Live { key: one, origin: 1 }, 2, 1, 60, 100, &first);
        routing.expression_on(ExpressionOwner::Live { key: two, origin: 2 }, 2, 1, 60, 100, &second);
        assert_eq!(routing.expressions(2).len(),2);
        let id_one = routing.expressions(2)[0].id();
        let id_two = routing.expressions(2)[1].id();
        assert_ne!(id_one,id_two);
        routing.next();
        let update = first.input(&[0xe1,0,96]);
        routing.expression_update(1,None,&first,update);
        assert_eq!(routing.expressions(2),[Event::Values { id: id_one, values: Values { pitch:24.0,pressure:0,timbre:64 }.into() }]);
        routing.next();
        let update = first.input(&[0xb0,64,127]);
        routing.expression_update(1,None,&first,update);
        routing.release(one);
        assert!(routing.expressions(2).is_empty());
        let update = first.input(&[0xd1,99]);
        routing.expression_update(1,None,&first,update);
        assert!(routing.expressions(2).is_empty(),"Released member changes cannot retarget a sustained voice");
        let update = first.input(&[0xd0,91]);
        routing.expression_update(1,None,&first,update);
        assert_eq!(routing.expressions(2),[Event::Values { id: id_one, values: Values { pitch:24.0,pressure:91,timbre:64 }.into() }]);
        routing.next();
        routing.expression_on(ExpressionOwner::Live { key: one, origin: 1 },2,1,60,100,&first);
        let reused = routing.expressions(2)[0].id();
        assert_ne!(reused,id_one);
        routing.next();
        let update = first.input(&[0xb0,64,0]);
        routing.expression_update(1,None,&first,update);
        assert_eq!(routing.expressions(2),[Event::Off { id:id_one,pitch:60 }]);
        routing.next();
        let update = second.input(&[0xd1,17]);
        routing.expression_update(2,None,&second,update);
        assert_eq!(routing.expressions(2),[Event::Values { id:id_two,values:Values {pitch:0.0,pressure:17,timbre:64}.into() }]);
        routing.next();
        routing.expression_retire(1);
        assert_eq!(routing.expressions(2),[Event::Off { id:reused,pitch:60 }]);
        routing.next();
        routing.release(two);
        assert_eq!(routing.expressions(2),[Event::Off { id:id_two,pitch:60 }]);
    });
    assert_eq!(counts, test_alloc::Counts::default());
}

#[test]
fn mpe_plugin_clip_note_ids_and_event_capacity_never_merge_live_voices() {
    use crate::engine::{mpe::State, midi_edit::NoteId};
    use crate::plugin_host::expression::Event;
    let mut routing=Routing::default();
    let mut state=State::default();
    for bytes in [[0xb0,101,0],[0xb0,100,6],[0xb0,6,4]] { state.input(&bytes); }
    let one=NoteId::new(); let two=NoteId::new();
    routing.expression_on(ExpressionOwner::Clip {note:one,track:2},2,1,60,100,&state);
    routing.expression_on(ExpressionOwner::Clip {note:two,track:2},2,2,60,100,&state);
    let id_one=routing.expressions(2)[0].id(); let id_two=routing.expressions(2)[1].id();
    routing.next(); routing.expression_off_clip(2,one);
    assert_eq!(routing.expressions(2),[Event::Off {id:id_one,pitch:60}]);
    routing.next();
    routing.expression_update(0,Some(3),&state,crate::engine::mpe::Update {silence:u16::MAX,..Default::default()});
    assert!(routing.expressions(2).is_empty());
    routing.clear_clip(2);
    assert_eq!(routing.expressions(2),[Event::Off {id:id_two,pitch:60}]);
    routing.next();
    for _ in 0..EVENTS { assert!(routing.push(2,[0xb0,7,100])); }
    routing.expression_on(ExpressionOwner::Clip {note:one,track:2},2,1,60,100,&state);
    assert!(routing.refused(2));
    assert!(routing.expressions(2).is_empty());
}
