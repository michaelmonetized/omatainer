#!/usr/bin/env python3
"""Read back every published ticket and verify its reviewed content and links."""
import collections
import datetime
import json
import re

import publish_backlog as publisher


def main():
    items, _ = publisher.load_items()
    pages = publisher.gh([
        'api', f'repos/{publisher.REPO}/issues?state=all&per_page=100',
        '--paginate', '--slurp',
    ])
    remote = {}
    for page in pages:
        for issue in page:
            body = issue.get('body') or ''
            if publisher.MARKER not in body:
                continue
            key = body.split(publisher.MARKER, 1)[1].split('-->', 1)[0].strip()
            assert key not in remote, f'Duplicate remote marker: {key}'
            remote[key] = issue
    expected = {item['id'] for item in items} | {'index'}
    assert set(remote) == expected, {
        'missing': sorted(expected - set(remote)),
        'unexpected': sorted(set(remote) - expected),
    }
    receipts = {
        key: {'id': key, 'number': issue['number'], 'url': issue['html_url'],
              'title': issue['title']}
        for key, issue in remote.items()
    }
    hub = receipts['index']
    for item in items:
        issue = remote[item['id']]
        assert issue['state'] == 'open', item['id']
        assert issue['title'] == item['title'], item['id']
        assert issue['body'] == publisher.make_body(item, receipts, hub), item['id']
        labels = {label['name'] for label in issue['labels']}
        assert {
            'bug' if item['kind'] == 'fix' else 'enhancement',
            'professional-parity', 'priority:' + item['priority'],
            'area:' + item['area'],
        } <= labels, item['id']
    assert remote['index']['body'] == publisher.index_body(items, receipts, True)
    listed = [int(number) for number in re.findall(
        r'^- \[ \] #(\d+) ', remote['index']['body'], re.MULTILINE)]
    assert len(listed) == len(items)
    assert set(listed) == {receipts[item['id']]['number'] for item in items}
    report = {
        'verified_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'repository': publisher.REPO,
        'tracking_issue': hub['url'],
        'verified_children': len(items),
        'verified_total': len(remote),
        'kinds': dict(collections.Counter(item['kind'] for item in items)),
        'areas': dict(collections.Counter(item['area'] for item in items)),
        'checks': [
            'Unique remote markers exactly match reviewed inventory',
            'All child titles, full bodies, dependency links and labels match',
            'All child issues are open',
            'Complete tracking index contains every child exactly once',
        ],
    }
    (publisher.ROOT / 'publication-verification.json').write_text(
        json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
