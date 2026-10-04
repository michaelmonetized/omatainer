#!/usr/bin/env python3
"""Exercise the shipped graph adapter against an owned private JACK or PipeWire server."""
import argparse, datetime as dt, hashlib, importlib.util, json, os, subprocess, time
from pathlib import Path

TEST = 'engine::audio::jack::native_tests::private_graph_loopback_quantum_reconnect_and_rate_change_retain_project'
INSPECT = '''import ctypes,json,sys
j=ctypes.CDLL("libjack.so.0")
j.jack_client_open.restype=ctypes.c_void_p;j.jack_client_open.argtypes=[ctypes.c_char_p,ctypes.c_uint,ctypes.POINTER(ctypes.c_uint)]
s=ctypes.c_uint();c=j.jack_client_open(b"OmatainerTestControl",3,ctypes.byref(s));assert c,s.value
j.jack_get_ports.argtypes=[ctypes.c_void_p,ctypes.c_char_p,ctypes.c_char_p,ctypes.c_ulong];j.jack_get_ports.restype=ctypes.POINTER(ctypes.c_char_p)
j.jack_get_buffer_size.argtypes=[ctypes.c_void_p];j.jack_get_buffer_size.restype=ctypes.c_uint
j.jack_get_sample_rate.argtypes=[ctypes.c_void_p];j.jack_get_sample_rate.restype=ctypes.c_uint
if len(sys.argv)>1:
 j.jack_set_buffer_size.argtypes=[ctypes.c_void_p,ctypes.c_uint];assert j.jack_set_buffer_size(c,int(sys.argv[1]))==0
p=j.jack_get_ports(c,None,None,0);names=[];i=0
while p and p[i]:names.append(p[i].decode());i+=1
print(json.dumps({'ports':names,'rate':j.jack_get_sample_rate(c),'quantum':j.jack_get_buffer_size(c)}))
j.jack_free.argtypes=[ctypes.c_void_p]
if p:j.jack_free(p)
j.jack_client_close.argtypes=[ctypes.c_void_p];assert j.jack_client_close(c)==0
'''

def stop(process):
    """Retire an owned child.
    Takes its process; returns after termination, escalating only that PID if needed.
    """
    if process is None: return
    if process.poll() is None: process.terminate()
    try: process.wait(timeout=5)
    except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)

def run(binary, destination, backend, prefix, test=TEST):
    """Qualify a native graph binary.
    Takes executable, new evidence directory, backend and private PipeWire package prefix; returns measured evidence.
    """
    destination.mkdir(); root=destination.resolve(); runtime=root/'runtime';runtime.mkdir(mode=0o700)
    env={**os.environ,'XDG_RUNTIME_DIR':str(runtime),'XDG_CONFIG_HOME':str(root/'config'), 'OMATAINER_NATIVE_GRAPH_DIR':str(root),'JACK_NO_AUDIO_RESERVATION':'1'}
    if backend=='jack':
        name=f'omatainer-133-{os.getpid()}';env['JACK_DEFAULT_SERVER']=name
    else:
        if prefix is None: raise ValueError('PipeWire verification requires an explicit private signed package prefix')
        env.update(LD_LIBRARY_PATH=str(prefix/'usr/lib'),PIPEWIRE_RUNTIME_DIR=str(runtime),PIPEWIRE_REMOTE='omatainer-test',
            PIPEWIRE_MODULE_DIR=str(prefix/'usr/lib/pipewire-0.3'),SPA_PLUGIN_DIR=str(prefix/'usr/lib/spa-0.2'),
            PIPEWIRE_CONFIG_DIR=str(prefix/'usr/share/pipewire'),PIPEWIRE_PROPS='{ jack.merge-monitor=false jack.short-name=true jack.filter-name=false node.autoconnect=false }')
    servers=[];child=None;started=dt.datetime.now(dt.timezone.utc).isoformat()
    def inspect(quantum=None):
        result=subprocess.run(['python3','-c',INSPECT,*([str(quantum)] if quantum else [])],env=env,capture_output=True,text=True,timeout=10,check=True)
        return json.loads(result.stdout)
    def start(rate):
        if backend=='jack':args=['jackd','-r','-n',name,'-d','dummy','-r',str(rate),'-p','128','-m']
        else:
            spec=importlib.util.spec_from_file_location('fixture',Path(__file__).with_name('check-audio-recovery.py'));fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)
            config=fixture.SERVER.replace('default.clock.rate = 48000',f'default.clock.rate = {rate} default.clock.allowed-rates = [ 44100 48000 ]')
            config=config.replace('monitor = false','monitor = true')
            config=config.replace('node.name = test-sink','node.name = test-sink node.description = "OmatainerFixture"')
            path=root/f'pipewire-{len(servers)}.conf';path.write_text(config);args=[str(prefix/'usr/bin/pipewire'),'-c',str(path)]
        log=(root/f'server-{len(servers)}.log').open('w');server=subprocess.Popen(args,env=env,stdout=log,stderr=log);log.close();servers.append(server)
        deadline=time.monotonic()+10
        while True:
            if server.poll() is not None:raise RuntimeError('Private server exited; inspect retained log')
            try:
                observed=inspect()
                if observed['rate']==rate:return server,observed
            except (subprocess.CalledProcessError,subprocess.TimeoutExpired):pass
            if time.monotonic()>deadline:raise TimeoutError('Private graph startup timed out')
            time.sleep(.1)
    try:
        server,inventory=start(48000)
        if backend=='jack':env.update(OMATAINER_GRAPH_OUTPUT='system:playback_1',OMATAINER_GRAPH_INPUT='system:monitor_1')
        else:
            output=[p for p in inventory['ports'] if p.endswith(':playback_FL')];capture=[p for p in inventory['ports'] if p.endswith(':monitor_FL')]
            if len(output)!=1 or len(capture)!=1:raise ValueError(f'Fixture endpoint inventory is ambiguous: {inventory}')
            env.update(OMATAINER_GRAPH_OUTPUT=output[0],OMATAINER_GRAPH_INPUT=capture[0])
        log=(root/'test.log').open('w');child=subprocess.Popen([str(binary),'--ignored','--exact',test,'--test-threads=1','--nocapture'],env=env,stdout=log,stderr=log);log.close()
        changed=False;restarted=False;deadline=time.monotonic()+180
        while child.poll() is None:
            if (root/'quantum.ready').exists() and not changed:
                if backend=='jack':inspect(256)
                else:subprocess.run([str(prefix/'usr/bin/pw-metadata'),'-r','omatainer-test','-n','settings','0','clock.force-quantum','256'],env=env,capture_output=True,timeout=5,check=True)
                (root/'quantum.changed').write_text('Private server buffer change requested\n');changed=True
            if (root/'restart.ready').exists() and not restarted:
                stop(server);time.sleep(.2);server,second=start(44100)
                (root/'restart.done').write_text(json.dumps(second));restarted=True
            if time.monotonic()>deadline:raise TimeoutError('Native graph qualification timed out')
            time.sleep(.02)
        if child.returncode:raise RuntimeError(f'Native test failed with exit {child.returncode}; see {root / "test.log"}')
        result=json.loads((root/'result.json').read_text())
        result.update(backend=backend,binary=str(binary),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),start_utc=started,finish_utc=dt.datetime.now(dt.timezone.utc).isoformat(),server_pids=[p.pid for p in servers],initial_inventory=inventory)
        (root/'receipt.json').write_text(json.dumps(result,indent=2)+'\n');return result
    finally:
        stop(child)
        for server in servers:stop(server)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--destination',required=True,type=Path);parser.add_argument('--backend',required=True,choices=['jack','pipewire']);parser.add_argument('--pipewire-prefix',type=Path);parser.add_argument('--test',default=TEST)
    args=parser.parse_args();print(json.dumps(run(args.binary.resolve(),args.destination,args.backend,args.pipewire_prefix,args.test),indent=2))
