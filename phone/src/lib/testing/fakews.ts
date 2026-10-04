// A WebSocket stand-in for unit tests: the test plays the relay, by hand.
import type { WsFactory, WsLike } from '../relaypool';

export class FakeWs implements WsLike {
  readyState = 0;
  sent: unknown[] = [];
  onopen: ((ev: any) => void) | null = null;
  onmessage: ((ev: any) => void) | null = null;
  onclose: ((ev: any) => void) | null = null;
  onerror: ((ev: any) => void) | null = null;

  constructor(readonly url: string) {}

  send(data: string) {
    this.sent.push(JSON.parse(data));
  }
  close() {
    this.readyState = 3;
  }

  /** The relay accepts the connection. */
  open() {
    this.readyState = 1;
    this.onopen?.({});
  }
  /** The relay sends a message. */
  push(msg: unknown) {
    this.onmessage?.({ data: JSON.stringify(msg) });
  }
  /** The connection drops. */
  drop() {
    this.readyState = 3;
    this.onclose?.({});
  }
  /** The messages of one type the page sent. */
  of(type: string): unknown[][] {
    return this.sent.filter((m) => Array.isArray(m) && m[0] === type) as unknown[][];
  }
}

/** A factory that keeps every socket it made, by relay address. */
export function fakeWsFactory(): { ws: WsFactory; sockets: FakeWs[]; last: (url: string) => FakeWs } {
  const sockets: FakeWs[] = [];
  return {
    sockets,
    ws: (url) => {
      const s = new FakeWs(url);
      sockets.push(s);
      return s;
    },
    last: (url) => [...sockets].reverse().find((s) => s.url === url)!,
  };
}

export const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));

/** Wait until `cond()` holds (polling), or fail after `ms`. */
export async function waitFor(cond: () => boolean, ms = 3000): Promise<void> {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error('timed out waiting');
    await tick(5);
  }
}
