"""Trace validation: check that executions of the Rust core are behaviors of spec/Tunnel.tla.

Usage (from the repo root): python spec/trace/check_traces.py [--spec PATH] [test-name ...]

--spec checks the traces against another copy of Tunnel.tla (e.g. a mutant, to confirm a
wrong model is rejected).

For each core unit test in TESTS (or the ones named), runs it with the `trace` feature so
the core writes an NDJSON trace (crates/outpunch/src/trace.rs), converts the trace into a
TLA+ module, and asks TLC whether some behavior of the model (spec/TunnelTrace.tla)
matches every event in order. On a mismatch it reports the first event the model cannot
take. Exit status is non-zero if any trace is rejected.

Only tests that drive the core through its public API are traced: tests that edit the
server's internal state directly (fake service handles) are not executions of the model.
"""

import json
import os
import shutil
import subprocess
import sys
from collections import Counter
from itertools import chain
from pathlib import Path

TESTS = [
    "handle_request_no_client_returns_502",
    "try_handle_request_not_connected",
    "auth_valid_token_registers_service",
    "auth_invalid_token_rejects",
    "auth_rejects_empty_token_when_secret_unset",
    "connection_rejects_non_auth_first_message",
    "connection_without_auth_is_closed",
    "connection_cleanup_removes_service",
    "connection_ignores_malformed_messages",
    "connection_routes_response_to_pending_request",
    "client_cannot_answer_another_services_request",
    "disconnect_mid_request_fails_fast_with_502",
    "stale_disconnect_does_not_remove_newer_connection",
    "newer_connection_replaces_and_closes_older_one",
    "request_times_out_over_a_real_connection",
    "connection_keeps_serving_after_a_request_is_cancelled",
    "dropped_run_unregisters_and_fails_pending",
    "dead_callback_unregisters_service",
    "silent_client_is_evicted_after_the_idle_timeout",
]

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "target" / "tlc" / "traces"
TLC = ROOT / "scripts" / "tla" / "tlc"


def record_trace(test, path):
    path.unlink(missing_ok=True)
    cmd = ["cargo", "test", "-q", "-p", "outpunch", "--features", "trace", "--lib", "--",
           "--exact", f"server::tests::{test}", "--test-threads=1"]
    run = subprocess.run(cmd, cwd=ROOT, env={**os.environ, "OUTPUNCH_TRACE": str(path)},
                         capture_output=True, text=True)
    if run.returncode != 0 or "1 passed" not in run.stdout:
        raise SystemExit(f"{test}: test did not pass\n{run.stdout}\n{run.stderr}")
    return [json.loads(line) for line in path.read_text().splitlines()]


def tla(value):
    if value is None:
        return "None"
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "{" + ", ".join(tla(v) for v in value) + "}"
    if isinstance(value, dict):
        return "[" + ", ".join(f"{k} |-> {tla(v)}" for k, v in value.items()) + "]"
    raise TypeError(value)


def numbered(prefix, keys):
    """Model value names in order of first appearance: {key: prefix1, ...}."""
    return {k: f"{prefix}{n}" for n, k in enumerate(dict.fromkeys(keys), 1)}


def model_message(msg, reqs, svcs):
    if msg["type"] == "auth":
        return {"type": "auth", "svc": svcs[msg["svc"]], "token": "right" if msg["token_ok"] else "wrong"}
    if msg["type"] == "resp" and msg["req"] in reqs:
        return {"type": "resp", "req": reqs[msg["req"]]}
    return {"type": "junk"}  # unknown request ids are ignored by the relay loop, like junk


def model_event(e, conns, reqs, svcs):
    fields = {
        "conn": lambda v: None if v is None else conns[v],
        "old": lambda v: None if v is None else conns[v],
        "req": lambda v: None if v is None else reqs[v],
        "failed": lambda v: sorted(reqs[r] for r in v),
        "unregistered": lambda v: v,
        "svc": lambda v: svcs[v],
        "ok": lambda v: v,
        "msg": lambda v: model_message(v, reqs, svcs),
    }
    return {"ev": e["ev"]} | {k: f(e[k]) for k, f in fields.items() if k in e}


def to_model(events):
    """Map ids to model values and messages to the model's message records."""
    servers = [e for e in events if e["ev"] == "server"]
    if len(servers) != 1:
        raise SystemExit(f"expected one server per test, got {len(servers)}")
    steps = [e for e in events if e["ev"] != "server"]
    conns = numbered("c", (e["conn"] for e in steps if e.get("conn") is not None))
    reqs = numbered("r", (e["req"] for e in steps if e["ev"] == "start"))
    svcs = numbered("s", chain(
        (e["svc"] for e in steps if "svc" in e),
        (e["msg"]["svc"] for e in steps if e["ev"] == "push" and e["msg"]["type"] == "auth"),
    ))
    log = [model_event(e, conns, reqs, svcs) for e in steps]
    pushes = Counter(m["conn"] for m in log if m["ev"] == "push")
    constants = {
        "Conns": sorted(conns.values()) or ["c1"],
        "Services": sorted(svcs.values()) or ["s1"],
        "Reqs": sorted(reqs.values()) or ["r1"],
        "SecretSet": servers[0]["secret_set"],
        "MaxMsgs": max(pushes.values(), default=0),
    }
    return log, constants


def write_check(test, log, constants, target, spec):
    d = WORK / test
    d.mkdir(parents=True, exist_ok=True)
    shutil.copy(spec, d / "Tunnel.tla")
    shutil.copy(ROOT / "spec" / "TunnelTrace.tla", d / "TunnelTrace.tla")
    body = ",\n    ".join(tla(m) for m in log)
    (d / "TraceCheck.tla").write_text(
        f"---- MODULE TraceCheck ----\nEXTENDS TunnelTrace\n\nLog == <<\n    {body}\n>>\n====\n")
    sets = lambda xs: "{" + ", ".join(json.dumps(x) for x in xs) + "}"
    cfg = d / f"{test}-{target}.cfg"
    cfg.write_text(f"""CONSTANTS
    Conns = {sets(constants["Conns"])}
    Services = {sets(constants["Services"])}
    Reqs = {sets(constants["Reqs"])}
    SecretSet = {tla(constants["SecretSet"])}
    MaxMsgs = {constants["MaxMsgs"]}
    Reconnects = FALSE
    RunDrops = TRUE
    DrainDies = TRUE
    Silences = TRUE
    None = None
    TraceLog <- Log
    Target = {target}

INIT TInit
NEXT TNext

INVARIANTS
    TraceSafety
    NotReached

CHECK_DEADLOCK FALSE
""")
    return d / "TraceCheck.tla", cfg


def matches(test, log, constants, target, spec):
    """'yes' if the first `target` events can be matched, 'no' if not, 'safety' if a property fails."""
    spec, cfg = write_check(test, log, constants, target, spec)
    out = subprocess.run([str(TLC), str(spec), str(cfg)], cwd=ROOT, capture_output=True, text=True).stdout
    if "Invariant NotReached is violated" in out:
        return "yes"
    if "Invariant TraceSafety is violated" in out:
        return "safety"
    if "No error has been found" in out:
        return "no"
    raise SystemExit(f"{test}: unexpected TLC output\n{out[-3000:]}")


def main():
    args = sys.argv[1:]
    spec = ROOT / "spec" / "Tunnel.tla"
    if args[:1] == ["--spec"]:
        spec, args = Path(args[1]).resolve(), args[2:]
    tests = args or TESTS
    WORK.mkdir(parents=True, exist_ok=True)
    rejected = 0
    for test in tests:
        log, constants = to_model(record_trace(test, WORK / f"{test}.ndjson"))
        result = matches(test, log, constants, len(log), spec)
        if result == "yes":
            print(f"ok    {test}: {len(log)} events")
            continue
        rejected += 1
        if result == "safety":
            print(f"FAIL  {test}: a safety property fails along the trace")
            continue
        lo, hi = 0, len(log)  # largest matched prefix by bisection
        while lo < hi:
            mid = (lo + hi + 1) // 2
            lo, hi = (mid, hi) if matches(test, log, constants, mid, spec) == "yes" else (lo, mid - 1)
        print(f"FAIL  {test}: model matches {lo} of {len(log)} events; cannot take event {lo + 1}: "
              f"{tla(log[lo])}")
    sys.exit(1 if rejected else 0)


if __name__ == "__main__":
    main()
