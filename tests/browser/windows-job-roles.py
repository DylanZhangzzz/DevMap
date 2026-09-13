"""Classify saved owned-Job command lines using Windows' argument parser."""
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import sys

shell = c.WinDLL('shell32'); kernel = c.WinDLL('kernel32')
shell.CommandLineToArgvW.argtypes = [w.LPCWSTR, c.POINTER(c.c_int)]
shell.CommandLineToArgvW.restype = c.POINTER(w.LPWSTR)
kernel.LocalFree.argtypes = [c.c_void_p]; kernel.LocalFree.restype = c.c_void_p


def canonical(value):
    return os.path.normcase(str(Path(value).resolve()).removeprefix('\\\\?\\'))


def argv(command):
    count = c.c_int(); pointer = shell.CommandLineToArgvW(command, c.byref(count))
    if not pointer:
        raise RuntimeError('CommandLineToArgvW failed')
    try:
        return [pointer[i] for i in range(count.value)]
    finally:
        kernel.LocalFree(pointer)


def analyze(job, worker):
    assert job['root_exit_code'] == 0 and job['empty_confirmed'] and not job['aborted']
    exe = canonical(worker['executable'])
    source = canonical(worker['source'])
    proxies = {p['pid'] for p in worker['cohorts']}
    cold = {p['pid'] for p in worker['cold']['samples']}
    roles, records = {}, []
    for row in job['resources']['samples']:
        for p in row['processes']:
            if p['pid'] in roles:
                continue
            role = 'test_worker' if p['pid'] == job['root_pid'] else 'helper'
            args = argv(p['command_line']) if p.get('command_line') else []
            if canonical(p['image']) == exe:
                assert args and canonical(args[0]) == exe, 'Missing/mismatched DevMap command'
                assert '--source' in args and canonical(args[args.index('--source')+1]) == source
                if p['pid'] in proxies | cold:
                    assert args[1] == 'mcp'
                    role = 'warm_proxy' if p['pid'] in proxies else 'cold_proxy'
                elif args[1:3] == ['runtime', '--identity']:
                    role = 'identity_helper'
                elif args[1] == 'runtime' and '--owner' in args and '--instance' in args:
                    role = 'owner_command'
                else:
                    raise AssertionError(f'Unknown DevMap role: {args}')
            roles[p['pid']] = role
            records.append({'pid': p['pid'], 'created_filetime': p['created_filetime'], 'role': role, 'argv': args})
    rows = []
    for sample in job['resources']['samples']:
        groups = {}
        for p in sample['processes']:
            role = roles[p['pid']]; group = groups.setdefault(role, {'count': 0, 'rss_bytes': 0})
            group['count'] += 1; group['rss_bytes'] += p['rss_bytes']
        rows.append({'elapsed_seconds': sample['elapsed_seconds'], 'complete': sample['complete_snapshot'], 'groups': groups})
    complete = [r for r in rows if r['complete'] and r['groups'].get('warm_proxy', {}).get('count') == 4]
    assert complete
    return {'scope': 'command roles within sampled owned Job; not IPC identity attestation or unsampled-lifetime proof',
            'records': records, 'rows': rows,
            'max_simultaneous_identity_helpers': max(r['groups'].get('identity_helper', {}).get('count', 0) for r in complete),
            'owner_pids': [r['pid'] for r in records if r['role'] == 'owner_command'],
            'max_simultaneous_owner_commands': max(r['groups'].get('owner_command', {}).get('count', 0) for r in complete)}


if __name__ == '__main__':
    directory = Path(sys.argv[1]).resolve()
    report = {}
    for side in ['baseline', 'candidate']:
        report[side] = analyze(json.loads((directory / f'{side}.job.json').read_text()),
                               json.loads((directory / f'{side}.worker.json').read_text()))
    with (directory / 'job-role-analysis.json').open('x', encoding='utf8') as output:
        json.dump(report, output, indent=2)
    print(json.dumps({side: {k: v for k, v in result.items() if k not in ['records', 'rows']} for side, result in report.items()}))
