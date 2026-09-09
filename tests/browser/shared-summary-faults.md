# Shared-summary transport failure checks

This script reuses `startOwner`, `proxy`, `initialize`, `required`, `sameOwner` and `stop` exported by the existing performance harness. It does not run a DevMap executable, create a Git repository, migrate a store or read a scale corpus. The transport context is a deliberately small mock fixture facade; these tests do not establish real receipt/SQL validation or the main benchmark's entire `finally` path.

Root runs it later with an explicit JSON file containing an existing `run_parent` and absolute `python` executable path:

```powershell
node tests/browser/shared-summary-faults.cjs --config C:/reviewed/fault-config.json
```

The parent creates one random retained run directory and extensionless Node mock programs named `mcp` and `runtime`. A child worker's cwd is that directory, so the real helpers can use their normal fixed argv against `process.execPath`. No global cwd/environment or production launch switch is changed. A Windows suspended-start kill-on-close Job wrapper owns the worker and mock descendants. The parent accepts success only after the wrapper reports an empty Job; timeout requests owned-job abort and preserves evidence. No PID-discovered kill or recursive deletion occurs.

Seven bounded cases:

1. An owned random pipe answers valid Hello. The real `startOwner` must reject occupancy before adding any child; the mock server remains listening until the test closes it.
2. A nonexistent executable produces an actual spawn failure. The child entry is retained, pending request rejects and exported cleanup completes.
3. A real mock stdio child returns a tool error. `required` must reject it while retaining an error sample and the complete result's byte count.
4. A real mock child withholds its response. The existing request timeout parameter is set to 100 ms; its actual pending entry must reject. This changes only a test call's existing transport timeout, not production deadlines.
5. A mock child exits with code 23 during a pending request. EOF/close rejects that request and the context records unexpected exit.
6. A mock owner is accepted through Hello with its true retained PID and requested instance, then exits with code 27. `sameOwner` must reject the now-dead owner.
7. Two real retained children are cleaned with `Promise.allSettled`. One child's local `kill` method is temporarily made to throw, proving that rejection does not skip cleanup of the other. The original method is restored and both children are actually reaped in `finally`. This is an explicit API-failure injection, not a claim to reproduce an OS kill-denial condition.

Cases retain incremental outcomes and a final worker report with child exits. The separate Job receipt proves normal worker/descendant completion when root executes the suite. The worker has a 60-second parent deadline; each endpoint/request/cleanup check is also bounded. All artifacts remain for review.

Implementation verification so far is `node --check` only. No worker, mock server or child process was executed while writing this slice. Root must run and review all seven outcomes before calling these fault paths validated. This does not replace real candidate startup/crash tests, full-scale latency/freshness/resource gates, or UI/host acceptance.
# Root execution update

On 2026-09-09 all seven cases passed against the exported helpers. The worker and every mock descendant were reaped with an empty Job receipt; the parent cleanup report contained no primary, transport or cleanup errors. This validates the documented mock transport scope, not corpus/SQL or real-owner performance acceptance.
