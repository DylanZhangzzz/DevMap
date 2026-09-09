"""Read one owned benchmark process through a retained Windows handle.

No process is started or stopped. Defaults to the ten-minute acceptance window.
The report distinguishes an exited owner from a continuously resident process.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import hashlib
import re
import os
from pathlib import Path
import time


class MemoryCounters(c.Structure):
    _fields_ = [("cb", w.DWORD), ("page_faults", w.DWORD)] + [
        (name, c.c_size_t) for name in (
            "peak_rss", "rss", "peak_paged", "paged", "peak_nonpaged",
            "nonpaged", "pagefile", "peak_pagefile", "private_bytes")]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--exe", type=Path)
    parser.add_argument("--seconds", type=float, default=600)
    parser.add_argument("--interval", type=float, default=5)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--owned-run-receipt", type=Path)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("this resource observer requires Windows")
    if not (0 < args.interval <= args.seconds <= 3600):
        parser.error("require 0 < interval <= seconds <= 3600")
    root = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    if output.exists():
        parser.error("output already exists")
    ownership = "legacy checkout target restriction"
    if args.owned_run_receipt:
        if args.self_test or not args.pid or not args.exe:
            parser.error("owned receipt requires explicit pid and exe")
        # The spawning Node parent is trusted to recheck its retained physical
        # directory identity. Python independently refuses reparse traversal,
        # restricts all files to that direct canonical run and verifies build.
        # This is not a claim of independent Python/Node inode equivalence.
        def no_reparse(path):
            if not path.is_absolute():
                parser.error("owned paths must be absolute")
            for part in [path, *path.parents]:
                if not os.path.lexists(part):
                    if part == path:
                        continue
                    parser.error("owned ancestor missing")
                stat = part.lstat()
                if part.is_symlink() or getattr(stat, "st_file_attributes", 0) & 0x400:
                    parser.error("owned path contains reparse point")
            return path.resolve()

        receipt_path = no_reparse(args.owned_run_receipt)
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
        if not re.fullmatch(r"[a-f0-9]{32}", receipt.get("nonce", "")):
            parser.error("invalid owned run nonce")
        run = no_reparse(Path(receipt["run"]["path"]))
        if not run.is_dir() or receipt_path.parent != run or receipt_path.name != "creation.json":
            parser.error("receipt must be the direct owned run creation file")
        expected = no_reparse(args.exe)
        output = no_reparse(args.output)
        if expected.parent != run or output.parent != run or not expected.is_file():
            parser.error("exe and output must be direct owned run files")
        if output in (expected, receipt_path) or output.exists():
            parser.error("output must be an exclusive new file")
        build = receipt.get("candidate_sha256", "")
        if not re.fullmatch(r"[a-f0-9]{64}", build):
            parser.error("missing recorded candidate SHA256")
        digest = hashlib.sha256()
        with expected.open("rb") as binary:
            for chunk in iter(lambda: binary.read(65536), b""):
                digest.update(chunk)
        if digest.hexdigest() != build:
            parser.error("candidate differs from owned run build")
        pid = args.pid
        ownership = "trusted Node parent physical-run verification; Python canonical/no-reparse/direct-file/build checks"
    else:
        if not output.is_relative_to(root / "target" / "verification"):
            parser.error("output must be under this checkout target/verification")
        if args.self_test:
            pid = os.getpid()
        else:
            if not args.pid or not args.exe:
                parser.error("pid and exe are required")
            expected = args.exe.resolve(strict=True)
            if not expected.is_relative_to(root / "target"):
                parser.error("benchmark executable must be under this checkout target")
            pid = args.pid

    kernel = c.WinDLL("kernel32", use_last_error=True)
    psapi = c.WinDLL("psapi", use_last_error=True)
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
    kernel.GetExitCodeProcess.restype = w.BOOL
    kernel.GetSystemTimeAsFileTime.argtypes = [c.POINTER(w.FILETIME)]
    kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
    kernel.GetProcessTimes.restype = w.BOOL
    kernel.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
    kernel.WaitForSingleObject.restype = w.DWORD
    kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
    kernel.QueryFullProcessImageNameW.restype = w.BOOL
    psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(MemoryCounters), w.DWORD]
    psapi.GetProcessMemoryInfo.restype = w.BOOL
    handle = kernel.OpenProcess(0x100000 | 0x1000 | 0x10, False, pid)
    if not handle:
        raise c.WinError(c.get_last_error())

    def checked(ok):
        if not ok:
            raise c.WinError(c.get_last_error())

    def ticks(value):
        return (value.dwHighDateTime << 32) | value.dwLowDateTime

    def sample():
        created, exited, system, user = (w.FILETIME() for _ in range(4))
        checked(kernel.GetProcessTimes(handle, c.byref(created), c.byref(exited), c.byref(system), c.byref(user)))
        wait = kernel.WaitForSingleObject(handle, 0)
        if wait not in (0, 258):
            raise c.WinError(c.get_last_error())
        alive = wait == 258
        memory = MemoryCounters()
        memory.cb = c.sizeof(memory)
        if alive:
            ok = psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb)
            if not ok and kernel.WaitForSingleObject(handle, 0) == 0:
                alive = False
            else:
                checked(ok)
        if not alive:
            checked(kernel.GetProcessTimes(handle, c.byref(created), c.byref(exited), c.byref(system), c.byref(user)))
        code = w.DWORD()
        checked(kernel.GetExitCodeProcess(handle, c.byref(code)))
        observed = w.FILETIME()
        kernel.GetSystemTimeAsFileTime(c.byref(observed))
        return {"elapsed_seconds": time.monotonic() - start, "alive": alive,
                "sampled_filetime": str(ticks(observed)), "exit_filetime": str(ticks(exited)),
                "exit_code": code.value if not alive else None,
                "created_filetime": ticks(created), "cpu_seconds": (ticks(system) + ticks(user)) / 10_000_000,
                "rss_bytes": memory.rss if alive else 0,
                "lifetime_peak_rss_bytes": memory.peak_rss if alive else None,
                "private_bytes": memory.private_bytes if alive else 0}

    try:
        image = c.create_unicode_buffer(32768)
        size = w.DWORD(len(image))
        checked(kernel.QueryFullProcessImageNameW(handle, 0, image, c.byref(size)))
        if not args.self_test and Path(image.value).resolve() != expected:
            raise RuntimeError("PID image does not match the explicit benchmark executable")
        start = time.monotonic()
        samples = [sample()]
        deadline = start + args.seconds
        while time.monotonic() < deadline:
            time.sleep(max(0, min(args.interval, deadline - time.monotonic())))
            samples.append(sample())
        elapsed = samples[-1]["elapsed_seconds"] - samples[0]["elapsed_seconds"]
        cpu = samples[-1]["cpu_seconds"] - samples[0]["cpu_seconds"]
        report = {"scope": "observer_self_test" if args.self_test else "owned_process_resource_window",
                  "pid": pid, "image": image.value, "requested_seconds": args.seconds,
                  "ownership_boundary": ownership,
                  "elapsed_seconds": elapsed, "ten_minute_window": args.seconds >= 600,
                  "mean_cpu_percent_one_core": 100 * cpu / elapsed,
                  "max_sampled_rss_bytes": max(s["rss_bytes"] for s in samples),
                  "lifetime_peak_rss_bytes": max((s["lifetime_peak_rss_bytes"] or 0) for s in samples),
                  "alive_for_entire_window": all(s["alive"] for s in samples),
                  "samples": samples,
                  "note": "Retained handle prevents PID reuse. Lifetime peak includes earlier startup. Exited-owner zero RSS is explicit, not a resident-process claim."}
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("x", encoding="utf-8") as stream:
            json.dump(report, stream, indent=2)
        print(json.dumps({k: v for k, v in report.items() if k != "samples"}))
    finally:
        kernel.CloseHandle(handle)


if __name__ == "__main__":
    main()
