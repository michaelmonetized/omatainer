"""Renderer-backed session boundaries and marks via native AT-SPI controls."""
def verify_history(named, action, state, wait_for, Atspi):
    def current(): return state()['session_history']
    def click(label):
        count = len(state()['actions'])
        action(named(label), 'click')
        wait_for(lambda: any(entry['action'] == 'Click' and entry['label'] == label
                            for entry in state()['actions'][count:]), 'history action: ' + label)
    click('History')
    wait_for(lambda: current()['open'] and current()['ready'], 'history store opens')
    click('Start session')
    wait_for(lambda: current()['active'] is not None and not current()['pending'], 'actual output start acknowledgement')
    start = current()['selected']
    assert start['state'] == 'active' and start['entries'], start
    assert any(entry['source']['kind'] == 'catalog' for entry in start['entries']), 'reopened project receipt lost catalog association'
    click('End session')
    wait_for(lambda: current()['active'] is None and not current()['pending'] and current()['durable'], 'actual output end and durable history save')
    ended = current()['selected']
    assert ended['state'] == 'ended' and ended['end_frame'] >= ended['start_frame']
    entry = ended['entries'][0]
    title = entry['source'].get('title', 'Unresolved track')
    deck = 'deck ' + chr(ord('A') + entry['deck'])
    mark = f"Mark unplayed · {title} · {deck} · entry {entry['id']}"
    click(mark)
    wait_for(lambda: not current()['pending'] and current()['selected']['entries'][0]['played_override'] is False, 'captured entry unplayed assertion')
    assert current()['selected']['entries'][0]['rates'] == entry['rates'], 'manual mark changed measured duration'
    for label in ['External track title', 'External track artist', 'History export destination']:
        field = named(label)
        assert field.get_role() == Atspi.Role.ENTRY
        assert field.get_text_iface() is not None
        assert field.get_component_iface().grab_focus()
        wait_for(lambda: state().get('focus') == label, 'history text focus: ' + label)
    click('Close history')
    wait_for(lambda: not current()['open'], 'history panel closes without changing session')
    return {'workflow': 'output-confirmed Start -> End -> durable save -> captured manual mark -> text focus -> close',
            'session_id': ended['id'], 'start_frame': ended['start_frame'], 'end_frame': ended['end_frame'],
            'typed_external_and_export': 'actual egui UI tests; pinned native adapter has no EditableText mutation',
            'physical_hardware_qa': False}
