#!/usr/bin/env python3
"""Copy the current repo, append invariant probes, run without audio or desktop startup.
Usage: python3 reproduce.py /path/to/omatainer [--abort-check]
Probes pass when the audited defect remains present. They are evidence, not fixes.
"""
import os, resource, shutil, signal, subprocess, sys, tempfile
from pathlib import Path
repo = Path(sys.argv[1]).resolve()
evidence = Path(__file__).resolve().parent
cache = repo / 'target'
with tempfile.TemporaryDirectory(prefix='omatainer-integration-audit-') as tmp:
    work = Path(tmp)
    shutil.copytree(repo / 'src', work / 'src')
    for name in ('Cargo.toml', 'Cargo.lock'):
        shutil.copy2(repo / name, work / name)
    for target, snippet in (('src/main.rs','ipc_audit_tests.rs'),('src/engine/midi.rs','midi_audit_tests.rs')):
        with (work / target).open('a') as f:
            f.write((evidence / snippet).read_text())
    cargo = ['cargo','test','--offline','--manifest-path',str(work/'Cargo.toml'),'--target-dir',str(cache),'--bin','omatainer','audit_','--','--nocapture','--test-threads=1']
    subprocess.run(cargo,check=True)
    if '--abort-check' in sys.argv[2:]:
        shutil.copy2(evidence/'audit_scene_crash.rs',work/'src/audit_scene_crash.rs')
        with (work/'Cargo.toml').open('a') as f:
            f.write('\n[[bin]]\nname="audit_scene_crash"\npath="src/audit_scene_crash.rs"\n')
        subprocess.run(['cargo','rustc','--offline','--manifest-path',str(work/'Cargo.toml'),'--target-dir',str(cache),'--bin','audit_scene_crash','--','-C','panic=abort'],check=True)
        resource.setrlimit(resource.RLIMIT_CORE,(0,0))
        result = subprocess.run([str(cache/'debug/audit_scene_crash')])
        print('scene 9 panic=abort return code:',result.returncode)
        assert result.returncode == -signal.SIGABRT
