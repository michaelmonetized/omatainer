"""Native AT-SPI analysis actions through the real private App/metadata/decoder."""
def verify_analysis(named, action, state, wait_for, Atspi):
    click = lambda label: action(named(label), 'click')
    row = state()['sampler_fixture_row']
    assert row is not None
    named('Crate selection').get_value_iface().set_current_value(row)
    click('analyze…')
    wait_for(lambda: state()['analysis']['open'], 'analysis panel opens')
    assert 'source' in named('Analyze selected row').get_description().lower()
    click('Analyze selected row')
    wait_for(lambda: not state()['analysis']['busy'] and state()['analysis']['waveform_bins'],
             'actual source analysis and catalog persistence')
    first = state()['analysis']
    assert '1 saved' in first['message'], first
    assert first['record']['duration']['value'] > 0
    click('Analyze selected row')
    wait_for(lambda: not state()['analysis']['busy'] and '1 reused from cache' in state()['analysis']['message'],
             'verified cached analysis avoids new decode')
    click('Inspect selected cache')
    wait_for(lambda: not state()['analysis']['busy'] and state()['analysis']['waveform_bins'],
             'saved waveform inspection')
    assert state()['analysis']['record'] == first['record']
    click('Close analysis panel')
    wait_for(lambda: not state()['analysis']['open'], 'analysis panel close')
    return {'workflow':'actual local FLAC -> analysis -> catalog save -> cache reuse -> inspection -> close',
            'waveform_bins':first['waveform_bins'],'duration':first['record']['duration']['value'],
            'physical_audio_qa':False}
