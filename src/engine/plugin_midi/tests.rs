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
