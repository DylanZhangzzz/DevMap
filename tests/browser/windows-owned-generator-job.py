"""Launch only the configured fixture test in a suspended, kill-on-close job.

stdin is an owner control pipe (EOF or any line requests abort). The test gets
NUL stdin and inherited stdout/stderr. No PID discovery or tree enumeration.
Optional planned teardown is for performance sample isolation, never proof of
natural descendant exit. The default strict lifecycle policy is unchanged.
"""
import argparse
import ctypes
import json
import os
import subprocess
import sys
import threading
import time
from ctypes import wintypes as w


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--teardown-descendants-after-success", action="store_true")
    parser.add_argument("args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    assert os.name == "nt"
    assert os.path.isabs(args.report) and not os.path.exists(args.report)
    assert os.path.isabs(args.exe)
    argv = args.args[1:] if args.args[:1] == ["--"] else args.args
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    size_t = ctypes.c_size_t

    class BasicLimit(ctypes.Structure):
        _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                    ("flags", w.DWORD), ("min_ws", size_t), ("max_ws", size_t),
                    ("active_limit", w.DWORD), ("affinity", size_t),
                    ("priority", w.DWORD), ("scheduling", w.DWORD)]

    class IoCounters(ctypes.Structure):
        _fields_ = [(name, ctypes.c_uint64) for name in
                    ["read_ops", "write_ops", "other_ops", "read_bytes", "write_bytes", "other_bytes"]]

    class ExtendedLimit(ctypes.Structure):
        _fields_ = [("basic", BasicLimit), ("io", IoCounters),
                    ("process_memory", size_t), ("job_memory", size_t),
                    ("peak_process_memory", size_t), ("peak_job_memory", size_t)]

    class Accounting(ctypes.Structure):
        _fields_ = [("user", ctypes.c_int64), ("kernel", ctypes.c_int64),
                    ("period_user", ctypes.c_int64), ("period_kernel", ctypes.c_int64),
                    ("faults", w.DWORD), ("total", w.DWORD),
                    ("active", w.DWORD), ("terminated", w.DWORD)]

    class Startup(ctypes.Structure):
        _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR),
                    ("title", w.LPWSTR), ("x", w.DWORD), ("y", w.DWORD),
                    ("x_size", w.DWORD), ("y_size", w.DWORD),
                    ("x_chars", w.DWORD), ("y_chars", w.DWORD),
                    ("fill", w.DWORD), ("flags", w.DWORD), ("show", w.WORD),
                    ("reserved_size", w.WORD), ("reserved_bytes", ctypes.c_void_p),
                    ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]

    class Process(ctypes.Structure):
        _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE),
                    ("pid", w.DWORD), ("tid", w.DWORD)]

    signatures = {
        "CreateJobObjectW": ([ctypes.c_void_p, w.LPCWSTR], w.HANDLE),
        "SetInformationJobObject": ([w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD], w.BOOL),
        "QueryInformationJobObject": ([w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD, ctypes.c_void_p], w.BOOL),
        "AssignProcessToJobObject": ([w.HANDLE, w.HANDLE], w.BOOL),
        "TerminateJobObject": ([w.HANDLE, w.UINT], w.BOOL),
        "CreateProcessW": ([w.LPCWSTR, w.LPWSTR, ctypes.c_void_p, ctypes.c_void_p, w.BOOL, w.DWORD, ctypes.c_void_p, w.LPCWSTR, ctypes.POINTER(Startup), ctypes.POINTER(Process)], w.BOOL),
        "ResumeThread": ([w.HANDLE], w.DWORD),
        "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
        "GetExitCodeProcess": ([w.HANDLE, ctypes.POINTER(w.DWORD)], w.BOOL),
        "TerminateProcess": ([w.HANDLE, w.UINT], w.BOOL),
        "GetStdHandle": ([w.DWORD], w.HANDLE),
        "CloseHandle": ([w.HANDLE], w.BOOL),
    }
    for name, (parameters, result) in signatures.items():
        getattr(k, name).argtypes = parameters
        getattr(k, name).restype = result

    def require(ok):
        if not ok:
            raise ctypes.WinError(ctypes.get_last_error())

    job = k.CreateJobObjectW(None, None)
    require(job)
    process = Process()
    assigned = False
    abort = threading.Event()
    report = {"schema": "devmap/owned-generator-job/1", "empty_confirmed": False}

    def active():
        accounting = Accounting()
        require(k.QueryInformationJobObject(job, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None))
        return accounting.active

    def empty():
        deadline = time.monotonic() + 5
        while active():
            if time.monotonic() >= deadline:
                raise TimeoutError("Owned job did not become empty")
            time.sleep(0.02)
        report["empty_confirmed"] = True

    try:
        limit = ExtendedLimit()
        limit.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; no breakaway.
        require(k.SetInformationJobObject(job, 9, ctypes.byref(limit), ctypes.sizeof(limit)))
        import msvcrt
        with open(os.devnull, "rb") as null:
            null_handle = msvcrt.get_osfhandle(null.fileno())
            os.set_handle_inheritable(null_handle, True)
            startup = Startup()
            startup.cb = ctypes.sizeof(startup)
            startup.flags = 0x100  # STARTF_USESTDHANDLES
            startup.stdin = null_handle
            startup.stdout = k.GetStdHandle(w.DWORD(-11).value)
            startup.stderr = k.GetStdHandle(w.DWORD(-12).value)
            command = ctypes.create_unicode_buffer(subprocess.list2cmdline([args.exe, *argv]))
            require(k.CreateProcessW(args.exe, command, None, None, True, 0x4 | 0x08000000,
                                     None, None, ctypes.byref(startup), ctypes.byref(process)))
        report["root_pid"] = process.pid
        require(k.AssignProcessToJobObject(job, process.process))
        assigned = True
        if k.ResumeThread(process.thread) == 0xFFFFFFFF:
            raise ctypes.WinError(ctypes.get_last_error())

        def watch_control():
            os.read(sys.stdin.fileno(), 1)  # EOF also means the owner disappeared.
            abort.set()

        threading.Thread(target=watch_control, daemon=True).start()
        while True:
            status = k.WaitForSingleObject(process.process, 50)
            if status == 0:
                report["aborted"] = False
                break
            if status != 0x102:
                raise ctypes.WinError(ctypes.get_last_error())
            if abort.is_set():
                report["aborted"] = True
                require(k.TerminateJobObject(job, 71))
                break
        if args.teardown_descendants_after_success:
            report["cleanup_policy"] = "planned_after_success"
            report["natural_lifecycle_acceptance"] = False
            report["planned_descendant_teardown"] = False
            # Only the signaled root's successful exit permits planned teardown.
            # An abort or failing worker must retain the strict failure path.
            if report.get("aborted") is False:
                finished_code = w.DWORD()
                require(k.GetExitCodeProcess(process.process, ctypes.byref(finished_code)))
                if finished_code.value == 0:
                    remaining = active()
                    report["active_processes_at_planned_teardown"] = remaining
                    if remaining:
                        require(k.TerminateJobObject(job, 71))
                        report["planned_descendant_teardown"] = True
        try:
            empty()
        except TimeoutError:
            # Permit accounting/normal descendant completion to settle, but do
            # not publish success for a remaining writer after the grace bound.
            report["descendants_after_root_exit"] = True
            require(k.TerminateJobObject(job, 71))
            empty()
        code = w.DWORD()
        require(k.GetExitCodeProcess(process.process, ctypes.byref(code)))
        report["root_exit_code"] = code.value
    except BaseException as error:
        report["error"] = repr(error)
        if process.process and not assigned:
            # The only process is still suspended, never allowed to create children.
            k.TerminateProcess(process.process, 71)
            if k.WaitForSingleObject(process.process, 5000) != 0:
                report["suspended_root_reap_failed"] = True
        if assigned:
            try:
                require(k.TerminateJobObject(job, 71))
                empty()
            except BaseException as cleanup_error:
                report["cleanup_error"] = repr(cleanup_error)
    finally:
        for handle in [process.thread, process.process, job]:
            if handle:
                k.CloseHandle(handle)
        with open(args.report, "x", encoding="utf8") as output:
            json.dump(report, output, indent=2)
    return 0 if report.get("empty_confirmed") and report.get("root_exit_code") == 0 and not any(
        report.get(key) for key in ["aborted", "error", "descendants_after_root_exit"]) else 71


if __name__ == "__main__":
    sys.exit(main())
