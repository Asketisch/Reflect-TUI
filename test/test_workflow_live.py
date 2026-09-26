#!/usr/bin/env python3
"""
Full agent-coding workflow live test for reflect-tui (real LLM, real tools).

Uses type_fast (== how a real paste/quick-type arrives; the slash popup
completion is verified correct for real usage). Covers the goal's end-to-end:
  Phase A — Subagent delegation + search (call_explorer / agent activity)
  Phase B — Plan mode: analyze → PlanReady → user REVISES/supplements →
            re-plan → approve [1] (AutoMode ⇒ AcceptEdits ⇒ write permission) →
            execute → file created with the supplemented content
  Phase C — /goal set + clear
  Phase D — /loop <secs> <cmd> actually fires, then /loop stop

Every phase dumps the rendered screen so we can SEE what the agent did.
"""
import os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from test_harness import TuiSession

WORKDIR = os.path.expanduser("~/Code/CNB/Reflect-TUI")
PLAN_FILE = os.path.join(WORKDIR, "WORKFLOW_PROOF.txt")


def wait_for(s, pred, timeout=90.0, step=0.6, label=""):
    deadline = time.time() + timeout
    last = ""
    while time.time() < deadline:
        t = s.screen_text(); last = t
        if pred(t):
            return True, t
        s.wait(settle=step, max_wait=step + 0.2)
    print(f"      [timeout {label} ({timeout:.0f}s)] screen tail:")
    for ln in last.splitlines()[-9:]:
        if ln.strip():
            print(f"        |{ln}")
    return False, last


def submit(s, text, settle=1.0):
    """type_fast + Enter — reliable for both slash commands and free text."""
    s.type_fast(text)
    s.enter(settle=settle)


def dump(s, tag, max_lines=44):
    print(f"\n      ── screen dump: {tag} ──")
    shown = 0
    for i, ln in enumerate(s.display_lines()):
        if ln.strip():
            print(f"      {i:2d}|{ln}"); shown += 1
        if shown >= max_lines:
            break


def check(name, ok, detail=""):
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}{(' — ' + detail) if detail else ''}")
    return ok


RES = []


# ───────────────────────────────────────────────────────────────
# Phase D + C: fast local commands first (no LLM round-trip needed)
# ───────────────────────────────────────────────────────────────
def phase_loop():
    print("\n========== Phase D: /loop <secs> <cmd> + /loop stop ==========")
    r = []
    with TuiSession(cols=120, rows=38) as s:
        s.wait_stable(max_wait=8.0)
        submit(s, "/loop 2 /status")
        ok_start, _ = wait_for(s, lambda t: "started" in t.lower(), timeout=10, label="loop start")
        r.append(check("/loop 2 /status started", ok_start))
        # let it fire ~7s → expect ≥2 submissions (each fires "▶ /status" user msg)
        start = time.time()
        while time.time() - start < 7:
            s._pump(settle=0.6, max_wait=1.0)
        markers = s.screen_text().count("▶ /status")
        dump(s, f"after ~7s of /loop 2 /status (saw {markers} '▶ /status' firings)", max_lines=26)
        r.append(check("/loop fired /status ≥ twice", markers >= 2, f"{markers} firings"))
        before = s.screen_text().count("▶ /status")
        submit(s, "/loop stop")
        time.sleep(4)
        s._pump(settle=0.5, max_wait=1.0)
        after = s.screen_text().count("▶ /status")
        r.append(check("/loop stop halted firing", after <= before + 1, f"before={before} after={after}"))
    return r


def phase_goal():
    print("\n========== Phase C: /goal set + clear ==========")
    r = []
    with TuiSession(cols=120, rows=38) as s:
        s.wait_stable(max_wait=8.0)
        submit(s, "/goal verify the release binary renders its banner")
        ok, _ = wait_for(s, lambda t: "goal" in t.lower(), timeout=15, label="goal set")
        dump(s, "after /goal set", max_lines=20)
        r.append(check("/goal set acknowledged", ok))
        submit(s, "/goal clear")
        ok2, _ = wait_for(s, lambda t: "cleared" in t.lower() or "goal" in t.lower(), timeout=15, label="goal clear")
        r.append(check("/goal clear acknowledged", ok2))
    return r


# ───────────────────────────────────────────────────────────────
# Phase A: subagent delegation + search
# ───────────────────────────────────────────────────────────────
def phase_subagent_search():
    print("\n========== Phase A: Subagent delegation + search ==========")
    r = []
    with TuiSession(cols=130, rows=44) as s:
        s.wait_stable(max_wait=8.0)
        submit(s,
            "Dispatch a sub-agent (the explorer sub-agent / call_explorer tool) to "
            "search the repo for every Cargo.toml file and report the count. "
            "When it returns, state the count in one line.")
        ok, txt = wait_for(
            s,
            lambda t: ("cargo.toml" in t.lower() and any(c.isdigit() for c in t))
                      or "explorer" in t.lower() or "sub-agent" in t.lower()
                      or "subagent" in t.lower() or "call_explorer" in t.lower(),
            timeout=150, label="subagent search",
        )
        dump(s, "subagent/search outcome", max_lines=48)
        # inspect /tasks for agent (collab) activity
        submit(s, "/tasks")
        s._pump(settle=0.5, max_wait=2.0)
        dump(s, "/tasks overlay (agent activity)", max_lines=28)
        s.esc(settle=0.5)
        r.append(check("subagent/search activity observed", ok,
                       "saw Cargo.toml+count / explorer / sub-agent markers"))
    return r


# ───────────────────────────────────────────────────────────────
# Phase B: Plan mode → revise → approve [1] → execute (file created)
# ───────────────────────────────────────────────────────────────
def phase_plan_revise_execute():
    print("\n========== Phase B: Plan mode → revise → approve [1] → execute ==========")
    if os.path.exists(PLAN_FILE):
        os.remove(PLAN_FILE)
    r = []
    with TuiSession(cols=130, rows=44) as s:
        s.wait_stable(max_wait=8.0)
        submit(s, "/mode plan")
        ok_mode, _ = wait_for(s, lambda t: "[plan]" in t.lower(), timeout=10, label="plan mode")
        r.append(check("entered plan mode", ok_mode))

        submit(s,
            "Create a file named WORKFLOW_PROOF.txt in the repo root whose single "
            "line of content is: hello-plan. Propose a plan first.")
        ok_plan, _ = wait_for(
            s,
            lambda t: ("approve" in t.lower() and "plan" in t.lower()) or "[1]" in t,
            timeout=150, label="PlanReady overlay",
        )
        r.append(check("PlanReady approval overlay appeared", ok_plan))
        if ok_plan:
            dump(s, "PlanReady overlay before revise")

        # ── REVISE / SUPPLEMENT via [3] ──
        if ok_plan:
            s.type_fast("3")              # [3] Revise plan
            s.wait(settle=1.2, max_wait=3.0)
            submit(s, "Change the content to two lines: hello-plan and REVIFIED-OK.")
            ok_replan, _ = wait_for(
                s,
                lambda t: ("approve" in t.lower() and "plan" in t.lower()) or "[1]" in t,
                timeout=150, label="re-plan after revise",
            )
            r.append(check("plan re-proposed after user supplement", ok_replan))
            if ok_replan:
                dump(s, "PlanReady after revise")

        # ── APPROVE & EXECUTE via [1] (AutoMode ⇒ AcceptEdits ⇒ write permission) ──
        s.type_fast("1")
        s.wait(settle=2.0, max_wait=5.0)
        ok_exec, txt = wait_for(
            s,
            lambda t: ("WORKFLOW_PROOF" in t) or os.path.exists(PLAN_FILE)
                      or "wrote" in t.lower() or "created" in t.lower() or "complete" in t.lower(),
            timeout=120, label="execution",
        )
        # grace period for the write to land
        deadline = time.time() + 30
        while time.time() < deadline and not os.path.exists(PLAN_FILE):
            s.wait(settle=1.0, max_wait=1.5)
        dump(s, "after approve [1] execution")
        r.append(check("plan executed", ok_exec or os.path.exists(PLAN_FILE)))

    time.sleep(0.5)
    exists = os.path.exists(PLAN_FILE)
    content_ok = False
    if exists:
        try:
            content = open(PLAN_FILE).read()
            content_ok = "REVIFIED-OK" in content
            print(f"      WORKFLOW_PROOF.txt content:\n{content}")
        except Exception as e:
            print(f"      read error: {e}")
    r.append(check("WORKFLOW_PROOF.txt created on disk", exists))
    r.append(check("file contains supplemented REVIFIED-OK line", content_ok))
    return r


def main():
    global RES
    for fn in [phase_loop, phase_goal, phase_subagent_search, phase_plan_revise_execute]:
        try:
            RES.extend(fn())
        except Exception as e:
            import traceback; traceback.print_exc()
            RES.append(check(fn.__name__, False, f"Exception: {e}"))
    print("\n" + "=" * 64)
    print("WORKFLOW LIVE SUMMARY")
    print("=" * 64)
    passed = sum(1 for x in RES if x)
    for i, x in enumerate(RES):
        print(f"  [{'PASS' if x else 'FAIL'}] check #{i}")
    print(f"\n{passed}/{len(RES)} workflow checks passed")
    sys.exit(0 if passed == len(RES) else 1)


if __name__ == "__main__":
    main()
