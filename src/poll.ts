// Runs a check every `intervalMs` while the window is focused. A check that
// comes due while the window is unfocused is skipped; the next focus then
// checks at once and restarts the interval from that moment.

export interface PollClock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

const systemClock: PollClock = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export class FocusPoller {
  private last = Number.NEGATIVE_INFINITY;
  private timer: unknown = null;
  private running = false;
  private started = false;

  constructor(
    private readonly check: () => Promise<void>,
    private readonly intervalMs: number,
    private focused: boolean,
    private readonly clock: PollClock = systemClock,
  ) {}

  // The first check runs right away when focused, else on the next focus.
  start(): void {
    if (this.started) return;
    this.started = true;
    if (this.focused) this.tick();
  }

  setFocused(focused: boolean): void {
    if (focused === this.focused) return;
    this.focused = focused;
    if (!this.started) return;
    if (!focused) {
      this.cancel();
      return;
    }
    const wait = this.last + this.intervalMs - this.clock.now();
    if (wait <= 0) this.tick();
    else this.schedule(wait);
  }

  private tick(): void {
    this.last = this.clock.now();
    this.schedule(this.intervalMs);
    // A slow check is never doubled up; the next one waits for its slot.
    if (this.running) return;
    this.running = true;
    this.check()
      .catch(() => {})
      .finally(() => (this.running = false));
  }

  private schedule(ms: number): void {
    this.cancel();
    this.timer = this.clock.setTimeout(() => {
      this.timer = null;
      if (this.focused) this.tick();
    }, ms);
  }

  private cancel(): void {
    if (this.timer === null) return;
    this.clock.clearTimeout(this.timer);
    this.timer = null;
  }
}
