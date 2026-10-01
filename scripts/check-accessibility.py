#!/usr/bin/env python3
"""Real egui -> AccessKit Unix -> AT-SPI -> egui/renderer check on private buses.

Requires Linux, dbus-run-session, at-spi2-core, Python GI Atspi/Gio, and the Rust
unit-test executable. Does not open a window or change desktop accessibility
settings. This is native API evidence, not an Orca or user-experience claim.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

CHILD='ui::atspi_tests::private_atspi_bridge_child'
PUMP=lambda:None

def wait_for(check, description, seconds=5):
    deadline=time.monotonic()+seconds
    last=None
    while time.monotonic()<deadline:
        PUMP()
        try:
            value=check()
            if value:return value
        except Exception as error:last=error
        time.sleep(.025)
    raise AssertionError(f'timed out: {description}; last error: {last}')

def outer(binary):
    with tempfile.TemporaryDirectory(prefix='omatainer-private-atspi-') as directory:
        root=Path(directory)
        for name in ['runtime','config','data','cache']:(root/name).mkdir(mode=0o700)
        (root/'private-harness').write_text('Private native accessibility fixture\n')
        env=os.environ.copy()
        for name in ['DBUS_SESSION_BUS_ADDRESS','AT_SPI_BUS_ADDRESS','DISPLAY','WAYLAND_DISPLAY','NO_AT_BRIDGE']:
            env.pop(name,None)
        env.update(OMATAINER_ATSPI_PRIVATE='1',OMATAINER_ATSPI_TEST_DIR=str(root),
                   XDG_RUNTIME_DIR=str(root/'runtime'),XDG_CONFIG_HOME=str(root/'config'),
                   XDG_DATA_HOME=str(root/'data'),XDG_CACHE_HOME=str(root/'cache'),GSETTINGS_BACKEND='memory')
        process=subprocess.Popen(['dbus-run-session','--',sys.executable,str(Path(__file__).resolve()),
                                  '--private','--test-binary',str(binary.resolve())],
                                 env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
        try:
            stdout,stderr=process.communicate(timeout=85)
            if process.returncode:
                raise RuntimeError(f'private AT-SPI fixture failed ({process.returncode}):\n{stdout}\n{stderr}')
            print(stdout.strip())
        finally:
            # This process group contains only the newly created private session.
            try:os.killpg(process.pid,signal.SIGTERM)
            except ProcessLookupError:pass
            if process.poll() is None:
                try:process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid,signal.SIGKILL);process.wait(timeout=2)

def private(binary):
    assert os.environ.get('OMATAINER_ATSPI_PRIVATE')=='1'
    root=Path(os.environ['OMATAINER_ATSPI_TEST_DIR'])
    assert (root/'private-harness').is_file()
    import gi
    gi.require_version('Gio','2.0');gi.require_version('Atspi','2.0')
    from gi.repository import Gio,GLib
    def pump():
        context=GLib.MainContext.default()
        for _ in range(100):
            if not context.pending():break
            context.iteration(False)
    global PUMP
    PUMP=pump
    session=Gio.bus_get_sync(Gio.BusType.SESSION,None)
    def call(bus,destination,path,interface,method,parameters=None,result=None):
        return bus.call_sync(destination,path,interface,method,parameters,
                             GLib.VariantType.new(result) if result else None,
                             Gio.DBusCallFlags.NONE,1500,None)
    def bus_call(bus,method,name):
        return call(bus,'org.freedesktop.DBus','/org/freedesktop/DBus',
                    'org.freedesktop.DBus',method,GLib.Variant('(s)',(name,)))
    daemon_fds=[];launcher=None;registry=None;child=None;Atspi=None
    launcher_log=open(root/'bus.log','w');child_log=open(root/'child.log','w')
    def remember(bus,name):
        pid=bus_call(bus,'GetConnectionUnixProcessID',name).unpack()[0]
        daemon_fds.append(os.pidfd_open(pid))
    def status(value):
        call(session,'org.a11y.Bus','/org/a11y/bus','org.freedesktop.DBus.Properties','Set',
             GLib.Variant('(ssv)',('org.a11y.Status','IsEnabled',GLib.Variant('b',value))))
    try:
        launcher=subprocess.Popen(['/usr/lib/at-spi-bus-launcher','--launch-immediately'],stdout=launcher_log,stderr=subprocess.STDOUT)
        wait_for(lambda:bus_call(session,'NameHasOwner','org.a11y.Bus').unpack()[0],'private accessibility bus owner')
        address=call(session,'org.a11y.Bus','/org/a11y/bus','org.a11y.Bus','GetAddress').unpack()[0]
        os.environ['AT_SPI_BUS_ADDRESS']=address
        accessibility=Gio.DBusConnection.new_for_address_sync(address,
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT|Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,None,None)
        remember(accessibility,'org.freedesktop.DBus')
        # Memory GSettings backend and isolated bus make this a private property
        # change, never a modification of the user's desktop preference.
        status(False)
        registry=subprocess.Popen(['/usr/lib/at-spi2-registryd'],stdout=launcher_log,stderr=subprocess.STDOUT)
        wait_for(lambda:bus_call(accessibility,'NameHasOwner','org.a11y.atspi.Registry').unpack()[0],'private accessibility registry')
        child=subprocess.Popen([str(binary.resolve()),'--exact',CHILD,'--ignored','--nocapture','--test-threads=1'],
                               stdout=child_log,stderr=subprocess.STDOUT)
        def state():
            if child.poll() is not None:raise RuntimeError('Rust bridge exited: '+(root/'child.log').read_text())
            return json.loads((root/'state.json').read_text())
        wait_for(lambda:state(),'first actual App frame')
        status(True)
        from gi.repository import Atspi
        Atspi.set_timeout(1000,2000)
        Atspi.init()
        desktop=Atspi.get_desktop(0)
        def application():
            for i in range(desktop.get_child_count()):
                app=desktop.get_child_at_index(i)
                if app.get_process_id()==child.pid:return app
        app=wait_for(application,'actual AccessKit application on private AT-SPI bus',8)
        remember(accessibility,'org.a11y.atspi.Registry')
        def tree_nodes():
            nodes={};pending=[app];visited=0
            while pending:
                node=pending.pop()
                # Menus and status nodes can disappear between child-count and
                # child lookup on this live tree. Required controls are still
                # checked through bounded named-node waits below.
                if node is None:continue
                visited+=1
                assert visited<=10_000,'unbounded accessibility fixture tree'
                name=node.get_name()
                if name:nodes[name]=node
                pending.extend(node.get_child_at_index(i) for i in range(node.get_child_count()))
            return nodes,visited
        def named(name):
            return wait_for(lambda:tree_nodes()[0].get(name),f'native node {name}')
        nodes,visited=tree_nodes()
        pitch=nodes['Deck A: Pitch'];platter=nodes['Deck A: Platter play or pause']
        pad=nodes['Sampler: Sample pad 1'];cue=nodes['Deck A: Hot cue 1']
        assert pitch.get_role()==Atspi.Role.SLIDER
        pitch_role=pitch.get_role_name()
        assert platter.get_role() in (Atspi.Role.PUSH_BUTTON,Atspi.Role.TOGGLE_BUTTON)
        value=pitch.get_value_iface();assert value is not None
        assert value.get_minimum_value()==-8 and value.get_maximum_value()==8
        assert pitch.get_component_iface().grab_focus()
        wait_for(lambda:state().get('focus')=='Deck A: Pitch','focus reached actual egui control')
        wait_for(lambda:pitch.get_state_set().contains(Atspi.StateType.FOCUSED),'AT-SPI focused state')
        assert value.set_current_value(-4.0)
        wait_for(lambda:state()['pitch']==.25,'SetValue reached actual renderer')
        verified_pitch=state()['pitch']
        wait_for(lambda:abs(value.get_current_value()+4)<1e-8,'numeric state returned through AT-SPI')
        def action(node,name):
            interface=node.get_action_iface();assert interface is not None
            names=[interface.get_action_name(i) for i in range(interface.get_n_actions())]
            index=next((i for i,actual in enumerate(names) if actual.lower()==name.lower()),None)
            assert index is not None,(node.get_name(),name,names)
            assert interface.do_action(index),(node.get_name(),name)
            return names
        playing=state()['playing'];platter_actions=action(platter,'click')
        wait_for(lambda:state()['playing']!=playing,'AT-SPI click reached deck transport')
        action(cue,'click');wait_for(lambda:state()['hotcue_1'],'hotcue click reached renderer')
        # accesskit_atspi_common 0.12 exports Click only, not CustomAction.
        # Exercise the production Actions button and ordinary menu entries so
        # this proves the actual Linux path, not a fixture-transformed tree.
        def alternate(node,label):
            name=node.get_name()
            assert node.get_component_iface().grab_focus()
            wait_for(lambda:state().get('focus')==name,f'focus for {name}')
            action(named('Actions for '+name),'click')
            return action(named(label),'click')
        cue_actions=alternate(cue,'Delete cue')
        wait_for(lambda:not state()['hotcue_1'],'Delete cue menu action reached renderer')
        pad_actions=alternate(pad,'Press pad')
        wait_for(lambda:state()['pad_held'] and state()['pad_sample_active'],
                 'pad menu press reached App gate and renderer sample voice')
        alternate(pad,'Release pad')
        wait_for(lambda:not state()['pad_held'],'pad menu release reached App gate')
        action(named('Sampler instrument'),'click')
        action(named('analog'),'click')
        wait_for(lambda:state()['sampler_instrument']=='analog','instrument choice reached renderer')
        synth_pad=named('Sampler: Pad 1: A MIDI note 57')
        alternate(synth_pad,'Press pad')
        wait_for(lambda:state()['pad_held'] and state()['held_pad_voices']==1,
                 'native menu press holds exactly one real synth voice')
        alternate(synth_pad,'Release pad')
        wait_for(lambda:not state()['pad_held'] and state()['held_pad_voices']==0,
                 'native menu release releases the real synth voice')
        # Complete native project workflow with the production menu, path
        # dialog's default filename, clip Actions and the same persisted codec.
        # Arbitrary path typing is covered separately by real egui key events.
        def menu(name,entry):
            action(named(name),'click')
            action(named(entry),'click')
        def project_ready():
            node=tree_nodes()[0].get('Project')
            return node and node.get_state_set().contains(Atspi.StateType.ENABLED)
        epoch=state()['undo_epoch']
        menu('Project','New project')
        action(named('Discard changes'),'click')
        wait_for(lambda:state()['undo_epoch']>epoch and state()['notes']==0,'native New installed an empty project')
        wait_for(project_ready,'New controls enabled after applied snapshot')
        clip=wait_for(lambda:next((node for name,node in tree_nodes()[0].items()
                                  if name.startswith('Clip track 1 scene 1:')),None),
                      'new project clip')
        alternate(clip,'Arm compose')
        wait_for(lambda:state()['compose_armed'],'native clip action armed composition')
        pad_name=wait_for(lambda:next((name for name in tree_nodes()[0]
                                     if name=='Sampler: Sample pad 1' or name.startswith('Sampler: Pad 1:')),None),
                          'new project pad')
        alternate(named(pad_name),'Press pad')
        wait_for(lambda:state()['pad_held'] and state()['notes']==1,'native pad composed a stored note')
        alternate(named(pad_name),'Release pad')
        wait_for(lambda:not state()['pad_held'],'native composition released its gate')
        action(named('Edit'),'click')
        undo=wait_for(lambda:next((node for name,node in tree_nodes()[0].items() if name.startswith('Undo ')),None),'named Undo command')
        action(undo,'click')
        wait_for(lambda:state()['notes']==0,'native Undo reverted composed note')
        action(named('Edit'),'click')
        redo=wait_for(lambda:next((node for name,node in tree_nodes()[0].items() if name.startswith('Redo ')),None),'named Redo command')
        action(redo,'click')
        wait_for(lambda:state()['notes']==1,'native Redo restored composed note')
        menu('Project','Save project as…')
        path_field=named('Project file path')
        assert path_field.get_text_iface() is not None,'project path has native text semantics'
        action(named('Save'),'click')
        wait_for(lambda:state()['saved_notes']==1,'native save persisted composed note')
        wait_for(project_ready,'save completed')
        epoch=state()['undo_epoch']
        menu('Project','New project')
        wait_for(lambda:state()['undo_epoch']>epoch and state()['notes']==0,'second native New installed empty state')
        wait_for(project_ready,'second New controls enabled')
        assert state()['all_pad_inputs_clear'],'project replacement clears all input ownership'
        saved_path=state()['project_file']
        action(named('Project'),'click')
        action(named('Open recent ⏵'),'click')
        action(named(saved_path),'click')
        wait_for(lambda:state()['notes']==1 and state()['undo_cursor']==0,'native recent reopen restored saved note with empty history')
        wait_for(project_ready,'reopened project controls enabled')
        action(named('Preferences'),'click')
        wait_for(lambda:state()['preferences_open'],'native Preferences window')
        scale=named('UI scale')
        assert scale.get_role()==Atspi.Role.SLIDER
        assert scale.get_value_iface().set_current_value(1.25)
        wait_for(lambda:state()['preferences_draft_scale']==1.25,'native settings value edits draft')
        assert state()['preferences_scale']==1.0,'draft changed live scale before Apply'
        action(named('Follow desktop theme and font'),'click')
        action(named('Preview changes'),'click')
        wait_for(lambda:'Preview ready' in state()['preferences_message'],'off-thread route preview')
        action(named('Apply and save'),'click')
        wait_for(lambda:state()['preferences_saved'] and state()['preferences_scale']==1.25,
                 'native Apply commits real private preferences file and applies scale')
        saved=json.loads((root/'preferences.json').read_text())
        assert saved['profiles'][saved['active']]['appearance']['scale']==1.25
        assert not state()['preferences_follow_theme']
        action(named('Cancel changes'),'click')
        wait_for(lambda:not state()['preferences_open'],'native preferences cancel closes editor')
        # The same native tree describes the control and drives the observed
        # lesson, including the held/released distinction. All work is inside
        # this fixture's private document, not the user's running application.
        pitch=named('Deck A: Pitch')
        pitch_description=pitch.get_description()
        assert 'Percent' in pitch_description and 'F2' in pitch_description,pitch_description
        action(named('Help'),'click')
        wait_for(lambda:state()['help_open'],'offline help opened')
        action(named('Record a held note'),'click')
        action(named('Start this lesson'),'click')
        wait_for(lambda:state()['help_lesson'] is not None,'lesson started')
        empty=wait_for(lambda:next((node for name,node in tree_nodes()[0].items()
                                  if name.startswith('Clip track 1 scene 3:')),None),'empty lesson cell')
        alternate(empty,'Arm compose')
        def lesson_next(step):
            wait_for(lambda:state()['help_lesson'][0]==step and state()['help_lesson'][1],f'lesson step {step} observed')
            action(named('Next lesson step'),'click')
            wait_for(lambda:state()['help_lesson'][0]==step+1,f'lesson next {step}')
        lesson_next(0)
        action(named('Sampler instrument'),'click');action(named('keys'),'click')
        wait_for(lambda:state()['sampler_instrument']=='keys','lesson keys instrument')
        lesson_pad=named('Sampler: Pad 1: A MIDI note 57')
        alternate(lesson_pad,'Press pad')
        lesson_next(1)
        assert not state()['help_lesson'][1],'held input falsely passed release step'
        alternate(lesson_pad,'Release pad');lesson_next(2)
        action(named('Disarm compose'),'click');lesson_next(3)
        recorded=wait_for(lambda:next((node for name,node in tree_nodes()[0].items()
                                  if name.startswith('Clip track 1 scene 3:')),None),'recorded lesson cell')
        alternate(recorded,'Launch once');lesson_next(4)
        assert state()['help_lesson'][2] and state()['lesson_notes']==1
        action(named('Cancel lesson'),'click')
        wait_for(lambda:state()['help_lesson'] is None,'cancel guide only')
        action(named('Enable performance mode'),'click')
        wait_for(lambda:state()['performance']['protected'],'native performance protection')
        action(named('Safe stop…'),'click')
        action(named('Stop all transports and release notes'),'click')
        wait_for(lambda:state()['performance']['stopped'],'native safe stop acknowledged')
        assert not state()['performance']['output_muted'],'safe stop silently became emergency mute'
        action(named('Recover inputs…'),'click')
        action(named('I have released the physical inputs'),'click')
        action(named('Inputs released — keep playback stopped'),'click')
        wait_for(lambda:not state()['performance']['recovery'],'native stopped input recovery')
        action(named('Emergency silence…'),'click')
        action(named('Confirm emergency silence'),'click')
        wait_for(lambda:state()['performance']['stopped'] and state()['performance']['output_muted'],
                 'native emergency mute acknowledged')
        action(named('Recover inputs…'),'click')
        action(named('I have released the physical inputs'),'click')
        action(named('Inputs released — keep output muted'),'click')
        wait_for(lambda:not state()['performance']['recovery'],'native keep-muted recovery')
        action(named('Leave performance mode…'),'click')
        action(named('Leave protection'),'click')
        wait_for(lambda:not state()['performance']['protected'],'native deliberate mode exit')
        assert state()['performance']['output_muted'],'mode exit cleared the emergency latch'
        result=state()
        expected={'Focus','SetValue','Click'}
        assert expected.issubset({a['action'] for a in result['actions']}),result
        assert result['frames']>4
        (root/'done').touch()
        child.wait(timeout=5)
        assert child.returncode==0,(root/'child.log').read_text()
        print(json.dumps({'platform':'Linux AT-SPI via private D-Bus','native_nodes_visited':visited,
                          'pitch_role':pitch_role,'pitch_range':[-8,8],
                          'pitch_renderer_after_native_setvalue':verified_pitch,'reopened_project_pitch':result['pitch'],'frames':result['frames'],
                          'preferences_saved_scale':result['preferences_scale'],'help_recording_lesson':'native arm -> held note -> release -> disarm -> launch','lesson_notes':result['lesson_notes'],'pitch_help':pitch_description,
                          'actions':result['actions'],'platter_actions':platter_actions,
                          'cue_actions':cue_actions,'pad_actions':pad_actions,
                          'alternate_action_path':'production Actions menu using native AT-SPI Click',
                          'project_workflow':'New -> compose -> Undo -> Redo -> Save -> New -> Open recent',
                          'persisted_notes':result['saved_notes'],'reopened_notes':result['notes'],
                          'performance_workflow':'enable -> safe stop -> explicit recovery -> emergency silence -> keep-muted recovery -> deliberate exit',
                          'performance':result['performance'],
                          'scope':'actual App/renderer plus native accessibility API; no window, Orca, desktop setting or hardware QA'},indent=2))
    except BaseException:
        launcher_log.flush();child_log.flush()
        print('Private bridge log:\n'+(root/'child.log').read_text(),file=sys.stderr)
        print('Private bus log:\n'+(root/'bus.log').read_text(),file=sys.stderr)
        raise
    finally:
        if Atspi is not None:Atspi.exit()
        # Kill owned daemon identities before waiting on process handles, so
        # cancellation cannot orphan a private bus while waiting on a child.
        for fd in reversed(daemon_fds):
            try:signal.pidfd_send_signal(fd,signal.SIGTERM)
            except ProcessLookupError:pass
            os.close(fd)
        for process in [child,registry,launcher]:
            if process is not None and process.poll() is None:
                process.terminate()
                try:process.wait(timeout=2)
                except subprocess.TimeoutExpired:process.kill();process.wait(timeout=2)
        launcher_log.close();child_log.close()

def main():
    # Both orchestration processes execute finally blocks if a caller cancels
    # with SIGTERM. The private process group is still killed by the outer one.
    def terminate(signum,frame):
        raise SystemExit(128+signum)
    signal.signal(signal.SIGTERM,terminate)
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary',type=Path,required=True)
    parser.add_argument('--private',action='store_true',help=argparse.SUPPRESS)
    args=parser.parse_args()
    if args.private:private(args.test_binary)
    else:outer(args.test_binary)
if __name__=='__main__':main()
