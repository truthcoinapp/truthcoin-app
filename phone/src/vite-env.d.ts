/// <reference types="svelte" />
/// <reference types="vite/client" />

/** True in `npm run build:local` and the tests: ws://127.0.0.1 and ws://localhost relays are allowed. */
declare const __LOCAL_RELAYS__: boolean;
/** The phone page's version (package.json). */
declare const __APP_VERSION__: string;
