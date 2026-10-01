"""Actual native sampler draft/decoder/audition/store/renderer workflow.

Called by the private AT-SPI harness. No hardware or user files are opened.
"""
def verify_sampler(named, action, state, wait_for, Atspi):
    def click(name):
        count = len(state()['actions'])
        action(named(name), 'click')
        wait_for(lambda: len(state()['actions']) > count, 'sampler native action: ' + name)
    row = state()['sampler_fixture_row']
    assert row is not None
    named('Crate selection').get_value_iface().set_current_value(row)
    before = state()['sampler_banks']
    click('Sampler: Edit sampler banks')
    wait_for(lambda: state()['sampler_editor']['open'], 'sampler editor opens')
    name = named('Sampler editor: New or reusable bank name')
    assert name.get_role() == Atspi.Role.ENTRY
    assert name.get_component_iface().grab_focus()
    assert 'separate' in name.get_description()
    click('Sampler editor: Create empty bank')
    wait_for(lambda: not state()['sampler_editor']['loading'] and
             state()['sampler_editor']['draft'] is not None and
             all(v is None for v in state()['sampler_editor']['draft']['frames']), 'empty preview prepared')
    click('Sampler editor: Assign selected local source')
    wait_for(lambda: not state()['sampler_editor']['loading'] and
             state()['sampler_editor']['draft']['frames'][0] is not None, 'actual FLAC decoded')
    gain = named('Sampler editor: Slot gain')
    assert gain.get_role() in (Atspi.Role.SLIDER, Atspi.Role.SPIN_BUTTON)
    assert gain.get_value_iface().set_current_value(0.6)
    wait_for(lambda: not state()['sampler_editor']['draft']['prepared'], 'gain draft changes')
    click('Sampler editor: Prepare slot preview')
    wait_for(lambda: state()['sampler_editor']['draft']['prepared'] and not state()['sampler_editor']['loading'], 'gain preview prepared')
    assert state()['sampler_banks'] == before, 'preview affected pads'
    click('Sampler editor: Audition selected slot')
    click('Sampler editor: Stop audition')
    wait_for(lambda: state()['sampler_editor']['audition'] is None, 'reserved audition stop')
    click('Sampler editor: Save reusable bank')
    wait_for(lambda: state()['sampler_editor']['saved'] and state()['sampler_editor']['durable'], 'actual reusable definition durable save')
    assert state()['sampler_banks'] == before, 'Save was incorrectly Apply'
    draft = state()['sampler_editor']['draft']
    click('Sampler editor: Apply bank')
    wait_for(lambda: not state()['sampler_editor']['applying'] and any(b['id'] == draft['id'] for b in state()['sampler_banks']), 'actual renderer Apply')
    applied = state()['sampler_banks']
    bank = next(b for b in applied if b['id'] == draft['id'])
    assert abs(bank['gain'] - 0.6) < 1e-6 and bank['frames'] == draft['frames'][0]
    click('Sampler editor: Edit current bank')
    click('Sampler editor: Clear selected slot')
    wait_for(lambda: not state()['sampler_editor']['loading'] and state()['sampler_editor']['draft']['frames'][0] is None, 'clear is preview only')
    click('Sampler editor: Cancel or close sampler editor')
    wait_for(lambda: not state()['sampler_editor']['open'], 'sampler draft cancelled')
    assert state()['sampler_banks'] == applied
    return {'workflow': 'create -> local FLAC -> gain -> prepare -> audition/stop -> reusable Save -> Apply -> clear preview -> Cancel',
            'applied_bank': bank, 'save_was_separate_from_apply': True, 'cancel_preserved_applied_bank': True}
