<script lang="ts">
  // Stroke icons, drawn on a 24px grid.
  let { name, size = 20 }: { name: string; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === "discover"}
    <circle cx="12" cy="12" r="9" /><path d="m15.5 8.5-2 5-5 2 2-5z" />
  {:else if name === "search"}
    <circle cx="11" cy="11" r="7" /><path d="m20 20-4-4" />
  {:else if name === "installed"}
    <path d="M12 3v12m0 0-4-4m4 4 4-4" /><path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />
  {:else if name === "back"}
    <path d="m15 18-6-6 6-6" />
  {:else if name === "windows"}
    <rect x="3" y="3" width="8" height="8" /><rect x="13" y="3" width="8" height="8" /><rect
      x="3"
      y="13"
      width="8"
      height="8"
    /><rect x="13" y="13" width="8" height="8" />
  {:else if name === "terminal"}
    <path d="m5 8 4 4-4 4" /><path d="M12 17h7" />
  {:else if name === "warning"}
    <path d="M12 3 2 20h20z" /><path d="M12 10v4" /><path d="M12 17h.01" />
  {:else if name === "refresh"}
    <path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" />
  {:else if name === "external"}
    <path d="M14 4h6v6" /><path d="M20 4 11 13" /><path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />
  {:else if name === "close"}
    <path d="M6 6l12 12M18 6 6 18" />
  {/if}
</svg>
