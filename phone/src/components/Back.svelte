<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  export let title: string;
  const dispatch = createEventDispatcher<{ back: null }>();
</script>

<div class="backbar">
  <button class="plain back" on:click={() => dispatch('back', null)} aria-label="Back">‹ Back</button>
  <h2>{title}</h2>
</div>

<style>
  .backbar {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0 0 12px -10px;
  }
  .back {
    color: var(--accent);
    font-size: 17px;
    min-width: 76px;
    flex: none;
  }
  h2 {
    min-width: 0;
  }
</style>
