// Text that came from elsewhere (a typed name, the desktop's answers) as the page shows it: control and format
// characters (bidi overrides, zero-width) taken out and the length capped, so nothing can disguise itself.

// Cc and Cf, plus U+2065 (unassigned, inside the invisible-operator block the desktop also strips).
const HIDDEN = /[\p{Cc}\p{Cf}⁥]/gu;
const FORMAT = /[\p{Cf}⁥]/gu;
const CONTROL = /\p{Cc}/gu;
const CONTROL_BUT_NEWLINE = /[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/g;

/**
 * A device name as both ends show it (PROTOCOL.md, "Pairing", 5): control and format characters removed, cut to 40
 * characters, trimmed; "Phone" when nothing is left. Matches the desktop's clean_name.
 */
export function cleanName(s: string): string {
  const kept = Array.from(String(s).replace(HIDDEN, '')).slice(0, 40).join('');
  return kept.trim() || 'Phone';
}

/**
 * Text from the desktop for display: format characters removed, control characters (tabs, line breaks unless
 * `multiline`) turned into spaces, at most `max` characters (with an ellipsis when cut).
 */
export function cleanText(s: unknown, max: number, multiline = false): string {
  if (typeof s !== 'string') return '';
  let t = s.replace(FORMAT, '').replace(multiline ? CONTROL_BUT_NEWLINE : CONTROL, ' ');
  t = multiline ? t.replace(/[ ]{2,}/g, ' ').replace(/\n{3,}/g, '\n\n') : t.replace(/\s{2,}/g, ' ');
  const chars = Array.from(t.trim());
  return chars.length > max ? chars.slice(0, max - 1).join('') + '…' : chars.join('');
}
