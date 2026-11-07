import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FocusPoller } from "./poll";

const MIN = 60_000;

describe("FocusPoller", () => {
  let calls: number[];
  let check: () => Promise<void>;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    calls = [];
    check = () => {
      calls.push(Date.now());
      return Promise.resolve();
    };
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("checks at start and then every interval while focused", async () => {
    const p = new FocusPoller(check, MIN, true);
    p.start();
    await vi.advanceTimersByTimeAsync(2 * MIN);
    expect(calls).toEqual([0, MIN, 2 * MIN]);
  });

  it("waits for focus when started unfocused, then checks at once", async () => {
    const p = new FocusPoller(check, MIN, false);
    p.start();
    await vi.advanceTimersByTimeAsync(5 * MIN);
    expect(calls).toEqual([]);
    p.setFocused(true);
    await vi.advanceTimersByTimeAsync(MIN);
    expect(calls).toEqual([5 * MIN, 6 * MIN]);
  });

  it("keeps the original slot when refocused before it came due", async () => {
    const p = new FocusPoller(check, MIN, true);
    p.start();
    await vi.advanceTimersByTimeAsync(30_000);
    p.setFocused(false);
    await vi.advanceTimersByTimeAsync(20_000);
    p.setFocused(true);
    await vi.advanceTimersByTimeAsync(MIN);
    expect(calls).toEqual([0, MIN]);
  });

  it("checks on focus after a missed slot and restarts the interval from there", async () => {
    const p = new FocusPoller(check, MIN, true);
    p.start();
    await vi.advanceTimersByTimeAsync(30_000);
    p.setFocused(false);
    await vi.advanceTimersByTimeAsync(170_000); // t = 200s, two slots missed
    expect(calls).toEqual([0]);
    p.setFocused(true);
    await vi.advanceTimersByTimeAsync(MIN);
    expect(calls).toEqual([0, 200_000, 260_000]);
  });

  it("does not check while unfocused", async () => {
    const p = new FocusPoller(check, MIN, true);
    p.start();
    p.setFocused(false);
    await vi.advanceTimersByTimeAsync(10 * MIN);
    expect(calls).toEqual([0]);
  });

  it("ignores repeated focus events and focus before start", async () => {
    const p = new FocusPoller(check, MIN, false);
    p.setFocused(true);
    p.setFocused(true);
    await vi.advanceTimersByTimeAsync(MIN);
    expect(calls).toEqual([]);
    p.start();
    p.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toEqual([MIN]);
  });

  it("never runs two checks at once and keeps going after a failure", async () => {
    let release: () => void = () => {};
    let n = 0;
    const slow = () => {
      n++;
      if (n === 1) return new Promise<void>((r) => (release = r));
      return Promise.reject(new Error("offline"));
    };
    const p = new FocusPoller(slow, MIN, true);
    p.start();
    await vi.advanceTimersByTimeAsync(MIN); // first still running: skipped
    expect(n).toBe(1);
    release();
    await vi.advanceTimersByTimeAsync(MIN); // runs, rejects
    await vi.advanceTimersByTimeAsync(MIN); // still scheduled after the failure
    expect(n).toBe(3);
  });
});
