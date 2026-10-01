"""Additional native AT-SPI beatgrid workflow for check-accessibility.py.

Uses only that harness's private App, buses, and renderer evidence. No desktop
configuration, physical controller, screen reader or tempo detector claims.
"""

def verify_grid(named, action, state, wait_for, Atspi):
    def click(name):
        count = len(state()['actions'])
        action(named(name), 'click')
        wait_for(lambda: len(state()['actions']) > count, 'native grid action: ' + name)
    if not state()['loaded']:
        click('Load selected crate item to deck A')
        wait_for(lambda: state()['loaded'], 'grid fixture loads current crate source')
    before = state()['grid']
    click('Deck A: Beatgrid editor')
    wait_for(lambda: state()['grid_editor'] is not None, 'native grid editor opens')
    origin = named('Deck A beatgrid: Downbeat seconds')
    assert origin.get_role() == Atspi.Role.ENTRY, origin.get_role_name()
    assert origin.get_component_iface().grab_focus()
    wait_for(lambda: state()['focus'] == 'Deck A beatgrid: Downbeat seconds',
             'native origin text field focus')
    tempo = named('Deck A beatgrid: Stretch tempo BPM')
    assert tempo.get_role() == Atspi.Role.ENTRY, tempo.get_role_name()
    half = named('Deck A beatgrid: Half tempo')
    assert half.get_role() == Atspi.Role.PUSH_BUTTON
    description = half.get_description()
    assert 'downbeat' in description and '20' in description, description
    initial = state()['grid_editor']['draft']
    assert initial is not None
    click('Deck A beatgrid: Half tempo')
    wait_for(lambda: state()['grid_editor']['draft']['seconds_per_beat'] == initial['seconds_per_beat'] * 2,
             'native half-tempo changes the preview')
    assert state()['grid'] == before, 'native preview modified renderer before Apply'
    click('Deck A beatgrid: Double tempo')
    wait_for(lambda: state()['grid_editor']['draft'] == initial, 'native double-tempo reverses half')
    click('Deck A beatgrid: Set downbeat at playhead')
    click('Deck A beatgrid: Slip +1 ms')
    draft = state()['grid_editor']['draft']
    assert state()['grid'] == before
    click('Deck A beatgrid: Apply grid')
    wait_for(lambda: state()['grid'] == draft and not state()['grid_editor']['pending'],
             'native Apply confirmed by actual renderer preparation')
    click('Deck A beatgrid: Reset manual grid')
    wait_for(lambda: state()['grid_editor']['draft'] is None, 'native reset is a draft')
    assert state()['grid'] == draft
    click('Deck A beatgrid: Cancel or close grid editor')
    wait_for(lambda: state()['grid_editor'] is None, 'native Cancel discards reset draft')
    assert state()['grid'] == draft
    return {'workflow': 'open -> focus -> half -> double -> set -> slip -> Apply -> reset -> Cancel',
            'applied': draft, 'half_tempo_help': description,
            'cancel_preserved_applied_grid': True}
