<script lang="ts">
  import type { Card } from "../api";
  import { openApp } from "../state.svelte";
  import AppIcon from "./AppIcon.svelte";
  import Badge from "./Badge.svelte";

  let { app }: { app: Card } = $props();
  const best = $derived(app.ways[0]);
</script>

<button class="card" onclick={() => openApp(app.id)}>
  <AppIcon src={app.icon} kind={best.kind} />
  <span class="text">
    <span class="name">{app.name}</span>
    <span class="summary">{app.summary}</span>
    <span class="ways">
      <Badge kind={best.kind} label={best.label} status={best.status} />
      {#if app.ways.length > 1}<span class="more">+{app.ways.length - 1} more</span>{/if}
    </span>
  </span>
</button>

<style>
  .card {
    display: flex;
    gap: 14px;
    align-items: flex-start;
    padding: 16px;
    border: 1px solid var(--line);
    border-radius: 16px;
    background: var(--surface);
    text-align: left;
    min-width: 0;
    transition: background 120ms, border-color 120ms;
  }
  .card:hover {
    background: var(--hover);
    border-color: var(--line-strong);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    font-size: 16px;
  }
  .summary {
    color: var(--muted);
    font-size: 13px;
    line-height: 1.4;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .ways {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .more {
    color: var(--faint);
    font-size: 12px;
  }
</style>
