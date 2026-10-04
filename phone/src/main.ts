import './app.css';
import App from './App.svelte';
import { takePairFragment } from './lib/fragment';

/** Inside another page's frame? Then that page could dress this one up, so it doesn't run there. */
function framed(): boolean {
  try {
    return window.top !== window.self;
  } catch {
    return true;
  }
}

let app: App | null = null;
if (framed()) {
  // Before the fragment or the storage is touched.
  const el = document.getElementById('app')!;
  el.textContent = 'Open this page directly: it does not run inside another page.';
} else {
  // Take the pairing fragment and remove it from the address bar before anything renders, so the one-time code
  // doesn't linger in history, bookmarks or screenshots.
  const pairValue = takePairFragment();
  app = new App({ target: document.getElementById('app')!, props: { pairValue } });
}
export default app;
