// Audio unlock — AudioContext.resume() must run inside (or traceable to) a
// user gesture or Chrome autoplay policy leaves the context suspended and
// the worklet never renders. The App registers the unlock callback on mount;
// connect handlers (WebSRT receive/publish) call it synchronously from their
// click handlers so the gesture survives the async WebTransport handshake.

let unlock: (() => void) | null = null;

export function registerAudioUnlock(fn: () => void): void {
  unlock = fn;
}

/** Call synchronously in any click handler that will lead to audio. */
export function userGestureUnlock(): void {
  unlock?.();
}
