"""Read-only snapshots of processes belonging to an already owned Windows Job.

No machine-wide enumeration or termination. Retained handles and membership
checks prevent PID reuse from admitting unrelated processes. RSS sums count
shared resident pages per process; sampled peaks can miss short-lived helpers.
"""
import ctypes as c
from ctypes import wintypes as w
import time


class Members(c.Structure):
    _fields_ = [('assigned', w.DWORD), ('count', w.DWORD), ('pids', c.c_size_t * 2048)]


class Memory(c.Structure):
    _fields_ = [('cb', w.DWORD), ('faults', w.DWORD)] + [
        (name, c.c_size_t) for name in ['peak_rss', 'rss', 'peak_paged', 'paged',
                                       'peak_nonpaged', 'nonpaged', 'pagefile',
                                       'peak_pagefile', 'private_bytes']]


class Sampler:
    def __init__(self, job):
        self.job, self.handles, self.samples = job, {}, []
        self.start = time.monotonic()
        self.k = c.WinDLL('kernel32', use_last_error=True)
        self.p = c.WinDLL('psapi', use_last_error=True)
        self.nt = c.WinDLL('ntdll')
        self.nt.NtQueryInformationProcess.argtypes = [w.HANDLE, w.ULONG, c.c_void_p, w.ULONG, c.POINTER(w.ULONG)]
        self.nt.NtQueryInformationProcess.restype = c.c_long
        signatures = {
            'QueryInformationJobObject': ([w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.c_void_p], w.BOOL),
            'OpenProcess': ([w.DWORD, w.BOOL, w.DWORD], w.HANDLE),
            'IsProcessInJob': ([w.HANDLE, w.HANDLE, c.POINTER(w.BOOL)], w.BOOL),
            'GetProcessTimes': ([w.HANDLE] + [c.POINTER(w.FILETIME)] * 4, w.BOOL),
            'QueryFullProcessImageNameW': ([w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)], w.BOOL),
            'WaitForSingleObject': ([w.HANDLE, w.DWORD], w.DWORD),
            'CloseHandle': ([w.HANDLE], w.BOOL),
        }
        for name, (args, result) in signatures.items():
            fn = getattr(self.k, name); fn.argtypes = args; fn.restype = result
        self.p.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]
        self.p.GetProcessMemoryInfo.restype = w.BOOL

    @staticmethod
    def require(ok):
        if not ok:
            raise c.WinError(c.get_last_error())

    def sample(self):
        members = Members()
        self.require(self.k.QueryInformationJobObject(self.job, 3, c.byref(members), c.sizeof(members), None))
        if members.count > 2048:
            raise RuntimeError('Job membership snapshot truncated')
        row = {'elapsed_seconds': time.monotonic() - self.start,
               'membership_count': members.count, 'assigned_count': members.assigned,
               'membership_counts_agree': members.count == members.assigned,
               'processes': [], 'unobserved': []}
        for pid in members.pids[:members.count]:
            try:
                if pid not in self.handles:
                    handle = self.k.OpenProcess(0x100000 | 0x1000 | 0x10, False, pid)
                    self.require(handle)
                    try:
                        member = w.BOOL()
                        self.require(self.k.IsProcessInJob(handle, self.job, c.byref(member)))
                        if not member.value:
                            raise RuntimeError('Process no longer belongs to owned Job')
                        image = c.create_unicode_buffer(32768); size = w.DWORD(len(image))
                        self.require(self.k.QueryFullProcessImageNameW(handle, 0, image, c.byref(size)))
                        self.handles[pid] = (handle, image.value, self.command_line(handle))
                    except BaseException:
                        self.k.CloseHandle(handle)
                        raise
                handle, image, command = self.handles[pid]
                if self.k.WaitForSingleObject(handle, 0) != 258:
                    raise RuntimeError('Process exited between membership and sample')
                created, exited, kernel, user = (w.FILETIME() for _ in range(4))
                self.require(self.k.GetProcessTimes(handle, c.byref(created), c.byref(exited), c.byref(kernel), c.byref(user)))
                memory = Memory(); memory.cb = c.sizeof(memory)
                self.require(self.p.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb))
                ticks = lambda f: (f.dwHighDateTime << 32) | f.dwLowDateTime
                row['processes'].append({'pid': pid, 'image': image, 'created_filetime': str(ticks(created)),
                                         'cpu_seconds': (ticks(kernel) + ticks(user)) / 10000000,
                                         'rss_bytes': memory.rss, 'private_bytes': memory.private_bytes, **command})
            except (OSError, RuntimeError) as error:
                row['unobserved'].append({'pid': pid, 'reason': str(error)})
        row['sum_rss_bytes'] = sum(p['rss_bytes'] for p in row['processes'])
        row['sum_private_bytes'] = sum(p['private_bytes'] for p in row['processes'])
        row['complete_snapshot'] = not row['unobserved'] and row['membership_counts_agree']
        self.samples.append(row)

    def command_line(self, handle):
        # Diagnostic only: native class 60 is not a portable application API.
        # https://github.com/winsiderss/phnt/blob/master/ntpsapi.h
        # Unsupported queries stay unknown; never infer a role from failure.
        class Unicode(c.Structure):
            _fields_ = [('length', w.USHORT), ('maximum', w.USHORT), ('buffer', c.c_void_p)]
        buffer = c.create_string_buffer(131072); returned = w.ULONG()
        status = self.nt.NtQueryInformationProcess(handle, 60, buffer, len(buffer), c.byref(returned))
        if status < 0:
            return {'command_line': None, 'command_line_error': f'NTSTATUS {status & 0xffffffff:08x}'}
        value = Unicode.from_buffer(buffer); begin = c.addressof(buffer)
        pointer = value.buffer or 0
        if value.length % 2 or not begin <= pointer <= pointer + value.length <= begin + len(buffer):
            return {'command_line': None, 'command_line_error': 'Invalid returned Unicode bounds'}
        return {'command_line': c.string_at(pointer, value.length).decode('utf-16-le')}

    def close(self):
        for handle, _, _ in self.handles.values():
            self.k.CloseHandle(handle)
        self.handles.clear()
        return {'scope': 'sampled owned Job including test worker; excludes external wrapper',
                'note': 'RSS sum double counts shared pages; short-lived helpers may be missed. Unobserved rows are not zero usage.',
                'samples': self.samples}
