interface LogPanel {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

interface Frames {
  request: (callback: FrameRequestCallback) => number;
  cancel: (id: number) => void;
  reducedMotion: () => boolean;
}

// Follow a moving target without restarting a browser smooth-scroll animation
// for every log entry. Only scrollTop changes, preserving horizontal position.
export function createLogScroller(getPanel: () => LogPanel | null, frames: Frames = {
  request: callback => window.requestAnimationFrame(callback),
  cancel: id => window.cancelAnimationFrame(id),
  reducedMotion: () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false,
}) {
  let frame: number | undefined;
  let lastTime: number | undefined;
  let lastWritten: number | undefined;
  let following = true;

  function stop() {
    if (frame !== undefined) frames.cancel(frame);
    frame = undefined;
    lastTime = undefined;
  }

  function tick(time: number) {
    frame = undefined;
    const panel = getPanel();
    if (!panel || !following) { lastTime = undefined; return; }
    const target = Math.max(0, panel.scrollHeight - panel.clientHeight);
    const delta = target - panel.scrollTop;
    const dt = Math.max(1, Math.min(64, time - (lastTime ?? time - 16)));
    lastTime = time;
    panel.scrollTop = frames.reducedMotion() || Math.abs(delta) <= 1
      ? target
      : panel.scrollTop + delta * (1 - Math.exp(-dt / 65));
    lastWritten = panel.scrollTop;
    if (Math.abs(target - panel.scrollTop) > 1) frame = frames.request(tick);
    else { panel.scrollTop = target; lastWritten = panel.scrollTop; lastTime = undefined; }
  }

  function append() {
    if (following && frame === undefined) frame = frames.request(tick);
  }

  function onScroll() {
    const panel = getPanel();
    if (!panel) return;
    // Scroll events caused by our own animation must not disable following.
    if (following && lastWritten !== undefined && Math.abs(panel.scrollTop - lastWritten) <= 1) return;
    following = panel.scrollHeight - panel.clientHeight - panel.scrollTop <= 8;
    if (!following) stop();
  }

  return {
    append,
    onScroll,
    reset() {
      stop();
      following = true;
      const panel = getPanel();
      if (panel) panel.scrollTop = 0;
      lastWritten = panel?.scrollTop;
    },
    dispose() { following = false; stop(); },
  };
}
