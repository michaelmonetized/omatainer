#!/usr/bin/env python3
"""Publish the reviewed manifests with durable, idempotent GitHub issue receipts.

Run from the repository checkout. Requires gh authentication and explicit authority
from the repository owner to create the complete backlog. Existing marked issues
are reused. No product code or pull requests are created.
"""
import argparse
import collections
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT=Path(__file__).resolve().parent
REPO='michaelmonetized/omatainer'
MARKER='<!-- omatainer-professional-backlog:'
FILES=('fixes.json','platform.json','production.json','dj.json')


def gh(args,payload=None):
    with tempfile.TemporaryDirectory(prefix='omatainer-issue-') as tmp:
        cmd=['gh',*args]
        if payload is not None:
            path=Path(tmp)/'payload.json';path.write_text(json.dumps(payload));cmd+=['--input',str(path)]
        proc=subprocess.run(cmd,text=True,capture_output=True,timeout=90)
        if proc.returncode:
            raise RuntimeError(proc.stderr.strip() or proc.stdout.strip())
        return json.loads(proc.stdout) if proc.stdout.strip() else None


def remote_inventory():
    result=gh(['api',f'repos/{REPO}/issues?state=all&per_page=100','--paginate','--slurp'])
    found={}
    for page in result:
        for item in page:
            body=item.get('body') or ''
            if MARKER in body:
                key=body.split(MARKER,1)[1].split('-->',1)[0].strip()
                if key in found:raise RuntimeError(f'Duplicate remote backlog key: {key}')
                found[key]={'id':key,'number':item['number'],'url':item['html_url'],'title':item['title']}
    return found


def load_items():
    items=[i for name in FILES for i in json.loads((ROOT/name).read_text())]
    keys={i['id'] for i in items}
    assert len(keys)==len(items),'duplicate manifest ids'
    assert len({i['title'].casefold() for i in items})==len(items),'duplicate titles'
    for i in items:
        assert i['title'].startswith(f"[{i['kind']}]/ "),i
        assert i['kind'] in ('fix','feat') and i['priority'] in ('P0','P1','P2','P3'),i['id']
        assert len(i['acceptance'])>=3 and i['evidence'] and i['validation'],i['id']
        assert all(dep in keys for dep in i.get('dependencies',[])),(i['id'],i.get('dependencies'))
    ordered=[];done=set();remaining=sorted(items,key=lambda i:(i['kind']!='fix',i['priority'],i['area'],i['id']))
    while remaining:
        ready=[i for i in remaining if set(i.get('dependencies',[]))<=done]
        if not ready:raise RuntimeError('Cyclic dependencies: '+','.join(i['id'] for i in remaining))
        for i in ready:ordered.append(i);done.add(i['id']);remaining.remove(i)
    return items,ordered


def save_receipts(receipts):
    temp=ROOT/'created-issues.json.tmp'
    temp.write_text(json.dumps(receipts,indent=2)+'\n')
    os.replace(temp,ROOT/'created-issues.json')


def make_body(i,receipts,hub):
    lines=[f"{MARKER} {i['id']} -->",f"**Priority:** {i['priority']} · **Area:** {i['area']} · **Tracking:** #{hub['number']}",'',i['problem'],'','**Evidence and scope**','']
    lines.extend('- '+s for s in i['evidence'])
    lines+=['','**Acceptance criteria**','']
    lines.extend('- [ ] '+s for s in i['acceptance'])
    lines+=['','**Validation**','',i['validation']]
    if i.get('dependencies'):
        lines+=['','**Dependencies**','']
        lines.extend(f"- #{receipts[d]['number']} — {receipts[d]['title']}" for d in i['dependencies'])
    lines+=['','Baseline: Omatainer `cae546c7bc840ec1a643e4befe18f91e86d0e4c7`; professional-capability review dated 2026-09-30. This issue is planned work, not a claim of shipped functionality.']
    body='\n'.join(lines)+'\n'
    assert len(body)<65000
    return body


def create(key,title,body,labels,receipts):
    payload={'title':title,'body':body,'labels':labels}
    for attempt in range(5):
        try:
            result=gh(['api',f'repos/{REPO}/issues','--method','POST'],payload)
            record={'id':key,'number':result['number'],'url':result['html_url'],'title':result['title']}
            receipts[key]=record;save_receipts(receipts)
            print(json.dumps({'created':record}),flush=True)
            time.sleep(1.1)
            return record
        except (RuntimeError,subprocess.TimeoutExpired) as exc:
            print(json.dumps({'error':str(exc),'id':key,'attempt':attempt+1}),flush=True)
            try:
                actual=remote_inventory()
                if key in actual:
                    receipts.update(actual);save_receipts(receipts);return actual[key]
            except Exception as inv:
                print(json.dumps({'reconcile_error':str(inv)}),flush=True)
            msg=str(exc).lower()
            if 'rate limit' in msg or 'abuse' in msg or 'secondary' in msg:
                for _ in range(2):time.sleep(30);print(json.dumps({'waiting':'GitHub rate-limit cooldown','id':key}),flush=True)
            elif 'timeout' in msg or '502' in msg or '503' in msg or 'connection' in msg:
                time.sleep(5*(attempt+1))
            else:raise
    raise RuntimeError('Retries exhausted for '+key)


def index_body(items,receipts,complete):
    counts=collections.Counter(i['kind'] for i in items)
    lines=[f'{MARKER} index -->','**Professional production and live DJ maturity backlog**','',f"{len(items)} separate capability/defect issues: {counts['fix']} fixes and {counts['feat']} features. {'Publication verified complete.' if complete else 'Publication in progress; this index is filled as individual issues are created.'}",'','Benchmarks checked on 2026-09-30: [Ableton Live 12.4.6](https://www.ableton.com/en/release-notes/live-12/) with Suite capabilities and identified optional Packs; [Serato DJ Pro 4.0.10](https://serato.com/dj/pro/downloads) with explicitly identified expansions. Live 12.4.15b5 and [Serato 5.0 public beta](https://the-drop.serato.com/announcements/introducing-lucid-beatgrids-in-serato-dj-5-0/) are distinguished from released functionality.','',
'P0 protects audio continuity, creative work and foundational operation. P1 covers essential professional workflows. P2 expands specialist workflows and ecosystem depth. P3 covers optional, platform-dependent or advanced capabilities. These priorities are sequencing proposals, not permission to omit scope.','',
'This is a broad, versioned coverage inventory, not a claim that every specialist has identical needs or that Omatainer already implements parity. Beyond-baseline capabilities are identified in their issues. Proprietary products/assets are comparison references; equivalent original or licensed implementation is expected. Streaming, hardware, SDK and format integrations must confirm actual support and rights.','',
'Each child issue contains its gap, evidence or benchmark, testable acceptance criteria and validation. Audited defect evidence separates reproduced behavior from source-only risks. No physical-controller, live-device underrun or end-to-end deployment results are fabricated. The existing 13 tests passed during the audit.','',
'**Delivery sequence**','',
'1. Fix crashes, silence/DC, stuck notes, timing, data loss and real-time hazards.','2. Establish persistence/recovery, audio I/O, recording/editing and reliable DJ preparation/performance.','3. Deliver mixing, automation, plugins, instruments/effects, hardware and library depth.','4. Validate specialty integrations, accessibility, exports, long-session reliability and compatibility.','']
    for kind in ('fix','feat'):
        for area in sorted({i['area'] for i in items if i['kind']==kind}):
            group=sorted((i for i in items if i['kind']==kind and i['area']==area),key=lambda i:(i['priority'],i['title']))
            lines += [f"**{'Fixes' if kind=='fix' else 'Features'} · {area} ({len(group)})**",'']
            for i in group:
                r=receipts.get(i['id'])
                lines.append(f"- [ ] #{r['number']} — {i['priority']} — {i['title']}" if r else f"- [ ] Pending — {i['priority']} — {i['title']}")
            lines.append('')
    body='\n'.join(lines)
    assert len(body)<65000,('index length',len(body))
    return body


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--publish',action='store_true');args=parser.parse_args()
    items,ordered=load_items()
    print(json.dumps({'reviewed':len(items),'kinds':dict(collections.Counter(i['kind'] for i in items)),'areas':dict(collections.Counter(i['area'] for i in items))}),flush=True)
    (ROOT/'all-issues.json').write_text(json.dumps(ordered,indent=2)+'\n')
    if not args.publish:return
    existing_labels={x['name'] for page in gh(['api',f'repos/{REPO}/labels?per_page=100','--paginate','--slurp']) for x in page}
    wanted={'professional-parity':('5319e7','Professional production and live DJ maturity inventory')}
    for priority,color in [('P0','b60205'),('P1','d93f0b'),('P2','fbca04'),('P3','c2e0c6')]:
        wanted['priority:'+priority]=(color,'Proposed professional backlog sequencing priority '+priority)
    for area in sorted({i['area'] for i in items}):
        wanted['area:'+area]=('bfdadc','Professional backlog area: '+area)
    for name,(color,description) in wanted.items():
        if name not in existing_labels:
            gh(['api',f'repos/{REPO}/labels','--method','POST'],{'name':name,'color':color,'description':description})
            print(json.dumps({'label_created':name}),flush=True);time.sleep(1.1)
    receipts=remote_inventory();save_receipts(receipts)
    key='index' 
    hub=receipts.get(key) or create(key,'[feat]/ Track professional producer and live DJ maturity',index_body(items,receipts,False),['enhancement','professional-parity'],receipts)
    for i in ordered:
        if i['id'] not in receipts:
            create(i['id'],i['title'],make_body(i,receipts,hub),['bug' if i['kind']=='fix' else 'enhancement','professional-parity','priority:'+i['priority'],'area:'+i['area']],receipts)
    actual=remote_inventory()
    missing=[i['id'] for i in items if i['id'] not in actual]
    if missing:raise RuntimeError('Missing remote issues: '+','.join(missing))
    receipts.update(actual);save_receipts(receipts)
    body=index_body(items,receipts,True)
    gh(['api',f"repos/{REPO}/issues/{hub['number']}",'--method','PATCH'],{'body':body})
    links=['**Published professional backlog**','',f"[GitHub tracking issue]({hub['url']})",'',f"{len(items)} child issues, independently verified through the GitHub API.",'']
    for i in sorted(items,key=lambda i:receipts[i['id']]['number']):
        r=receipts[i['id']];links.append(f"- [#{r['number']} {i['title']}]({r['url']}) — {i['priority']} / {i['area']}")
    (ROOT/'published-issues.md').write_text('\n'.join(links)+'\n')
    print(json.dumps({'complete':len(items),'index':hub,'kinds':dict(collections.Counter(i['kind'] for i in items))}),flush=True)

if __name__=='__main__':main()
