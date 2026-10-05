"""Exercise waveform zoom through the private native AT-SPI App."""

def verify_waveform(named, action, state, wait_for):
    """Check native zoom controls.
    Takes private node, action and state helpers; returns observed linked and independent views.
    """
    def click(name):
        count = len(state()['actions'])
        action(named(name), 'click')
        wait_for(lambda: len(state()['actions']) > count, 'native waveform action: ' + name)
    for _ in range(4):
        if state()['waveform_view']['zoom'] == ['sixteen_bars'] * 2:
            break
        click('Deck A: Waveform zoom')
    wait_for(lambda: state()['waveform_view']['zoom'] == ['sixteen_bars'] * 2, 'linked waveform zoom')
    linked = state()['waveform_view']
    click('Linked waveform zoom')
    click('Deck B: Waveform zoom')
    wait_for(lambda: state()['waveform_view']['zoom'] == ['sixteen_bars', 'two_bars'], 'independent waveform zoom')
    independent = state()['waveform_view']
    description = named('Deck A: Waveform position').get_description()
    assert 'renderer position' in description and 'Phrase' in description, description
    click('Linked waveform zoom')
    wait_for(lambda: state()['waveform_view']['linked'], 'relinked waveform zoom')
    assert len(set(state()['waveform_view']['zoom'])) == 1
    return {'linked': linked, 'independent': independent, 'position_description': description}
