"""Synthetic evidence ONLY for validator/transaction tests, never release QA."""
import importlib.util
import json
from pathlib import Path

SPEC=importlib.util.spec_from_file_location('fixture_gate',Path(__file__).with_name('performance-gate.py'))
gate=importlib.util.module_from_spec(SPEC);SPEC.loader.exec_module(gate)

def rules(target="x86_64-unknown-linux-gnu"):
    return {'schema':1,'suite':'supported-workloads-v1','status':'reviewed',
            'test':'private_fixture::not_a_real_benchmark','timeout_seconds':1,
            'targets':[target],
            'scope':'SYNTHETIC VALIDATOR FIXTURE; not performance evidence',
            'unresolved_limits':['Synthetic numbers only; never use for a release.'],
            'workloads':[{'id':name,'conditions':{'fixture':True},'repetitions':3,
              'samples':{'frame_wall_ns':{'count':3,'max':{'p99':30,'max':30,'spread':20,'p99_minus_p50':10}}},
              'metrics':{'allocations':{'unit':'count','min':0,'max':0}},
              'checks':['finite_output'],'expected_observations':{target:{'state_hash':'private-fixture-state'}}} for name in gate.WORKLOADS]}

def raw(policy=None,manifest=None):
    policy=policy or rules()
    return {'schema':1,'suite':policy['suite'],'embedded_manifest':manifest or {},'workloads':[
        {'id':w['id'],'conditions':w['conditions'],'measurements':[
            {'samples':{'frame_wall_ns':[10,20,30]},'metrics':{'allocations':0},
             'checks':{'finite_output':True},'observations':{'state_hash':'private-fixture-state'}}
            for _ in range(w['repetitions'])]} for w in policy['workloads']]}

def report(root,binary):
    root=Path(root)
    manifest=json.loads((root/'licenses/manifest.json').read_text())
    target=manifest['target'];arch=target.split('-')[0]
    policy=rules(target);policy_text=gate.encode(policy).decode()
    (root/gate.POLICY).parent.mkdir(parents=True,exist_ok=True)
    (root/gate.POLICY).write_text(policy_text)
    manifest=json.loads((root/'licenses/manifest.json').read_text())
    manifest['source_files'][gate.POLICY]=gate.sha(policy_text.encode())
    (root/'licenses/manifest.json').write_bytes(gate.encode(manifest))
    sample=raw(policy,manifest);summary,failures=gate.evaluate(sample,policy,target,manifest)
    result={'schema':1,'status':'pass','started_utc':'2026-10-01T00:00:00+00:00','finished_utc':'2026-10-01T00:00:01+00:00',
            'host':{'execution':'local','system':'Linux','architecture':arch,'kernel':'fixture','cpu_model':'SYNTHETIC TEST FIXTURE','logical_cpus':1,'affinity':[0],'memory_bytes':1,'governors':[],'load_average_before_workload':[0,0,0],'load_average_after_workload':[0,0,0],'process_nice':0,'scheduler_policy':0},
            'build':{'profile':'release','cargo':'fixture','binary_info':{'schema':1,'debug_assertions':False,'pkg_version':manifest['application'],'arch':arch,'os':'linux'},
                     'environment':{},'test_exit_code':0,'test_binary_sha256':'0'*64,'native_accessibility':{'exit_code':0,'report':{'platform':'Linux AT-SPI via private D-Bus','native_nodes_visited':1,'frames':5,'pitch_role':'slider','pitch_range':[-8,8],'pitch_renderer_after_native_setvalue':.25,'persisted_notes':1,'reopened_notes':1,'preferences_saved_scale':1.25,'actions':[{'action':action} for action in ('Focus','SetValue','Click')],'scope':'SYNTHETIC validator fixture; not native evidence'}}},
            'bindings':gate.bindings(root,Path(binary),manifest),'policy_text':policy_text,'raw':sample,'summary':summary,'failures':failures}
    gate.atomic(root/gate.REPORT,gate.encode(result))
    return result
