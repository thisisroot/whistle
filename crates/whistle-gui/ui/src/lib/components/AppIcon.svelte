<script lang="ts">
  import type { Kind } from "../api";
  import Icon from "./Icon.svelte";

  // The app's icon; Windows apps and apps without one get a drawn tile.
  let { src, kind, size = 56 }: { src: string; kind: Kind; size?: number } = $props();
  let broken = $state(false);
</script>

{#if src && !broken}
  <img {src} alt="" width={size} height={size} loading="lazy" onerror={() => (broken = true)} />
{:else}
  <span class="tile" style:width="{size}px" style:height="{size}px" style:border-radius="{size / 4}px">
    <Icon name={kind === "windows" ? "windows" : "terminal"} size={Math.round(size / 2)} />
  </span>
{/if}

<style>
  img {
    flex: none;
    object-fit: contain;
  }
  .tile {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--tile);
    color: #dce1ea;
  }
</style>
