// Node 18 has no global WebCrypto in ESM; Node >= 20 does.
import { webcrypto } from 'node:crypto';
if (!globalThis.crypto?.subtle) {
  Object.defineProperty(globalThis, 'crypto', { value: webcrypto, configurable: true });
}
