"""Actual named-crate edits through the private native AT-SPI App bridge."""
def verify_crates(named, action, state, wait_for, Atspi):
    def current(): return state()['named_crates']
    def click(label):
        count = len(state()['actions'])
        action(named(label), 'click')
        wait_for(lambda: any(entry['action'] == 'Click' and entry['label'] == label
                            for entry in state()['actions'][count:]), 'crate action: ' + label)
    def settle():
        wait_for(lambda: not current()['pending'] and not current()['owner_active'], 'actual crate save receipt')
        assert current()['durable'], current()
    def select(row):
        assert named('Crate tree row (0 = All tracks)').get_value_iface().set_current_value(row)
        wait_for(lambda: (current()['selected'] is None) == (row == 0), 'crate selection')
    def node(identity):
        return next(n for n in current()['forest']['nodes'] if n['id'] == identity)

    click('Choose or edit named crates')
    wait_for(lambda: current()['open'], 'crate manager opens')
    field = named('Crate name')
    assert field.get_role() == Atspi.Role.ENTRY
    assert field.get_component_iface().grab_focus()
    wait_for(lambda: state().get('focus') == 'Crate name', 'crate name native focus')
    # The pinned adapter exposes text/focus, not native EditableText mutation.
    # The ordinary product draft supplies New crate; every mutation below is native.
    click('New root crate'); settle()
    root = current()['selected']
    assert root and node(root)['name'] == 'New crate', (current(), state()['actions'][-5:])
    click('Use as destination'); select(0)
    click('Add filtered tracks'); settle()
    original = node(root)['members']
    assert len(original) >= 3
    select(1)
    wait_for(lambda: current()['selected'] == root, 'root selected')
    click('New child crate'); settle()
    child = current()['selected']
    assert child != root and node(root)['children'] == [child]
    click('Use as destination'); select(1)
    wait_for(lambda: current()['selected'] == root, 'source crate selected')
    click('Toggle member at row')
    click('Copy selected to destination'); settle()
    assert node(child)['members'] == [original[0]] and node(root)['members'] == original
    assert named('Insert before row (last + 1 appends)').get_value_iface().set_current_value(len(original) + 1)
    wait_for(lambda: current()['insert_position'] == len(original) + 1, 'manual order anchor')
    click('Reorder selected members'); settle()
    reordered = original[1:] + original[:1]
    assert node(root)['members'] == reordered
    select(2)
    wait_for(lambda: current()['selected'] == child, 'child selected')
    click('Delete crate…'); click('Keep crate')
    assert node(child)['members'] == [original[0]]
    click('Delete crate…'); click('Confirm delete crate subtree'); settle()
    assert all(n['id'] != child for n in current()['forest']['nodes'])
    assert node(root)['members'] == reordered
    select(0); click('Close crates')
    wait_for(lambda: not current()['open'], 'crate manager closes')
    return {'workflow':'create root/child -> overlapping copy -> manual reorder -> cancel deletion -> delete child only',
            'root':root,'manual_order':reordered,'saved':True,
            'name_entry':'ordinary New crate draft; typed rename covered by actual egui event tests',
            'physical_controller_qa':False}
