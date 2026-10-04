<script lang="ts">
  // The app's mark: a dial, 62% filled.
  export let size = 28;
  const r = 11;
  const c = 2 * Math.PI * r;
</script>

<svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <circle cx="16" cy="16" {r} fill="none" stroke="var(--dial-rest)" stroke-width="7" />
  <circle
    cx="16"
    cy="16"
    {r}
    fill="none"
    stroke="var(--accent)"
    stroke-width="7"
    stroke-dasharray="{c * 0.62} {c}"
    transform="rotate(-90 16 16)"
  />
</svg>
