<script lang="ts">
  import type { Card } from "../api";
  import { openApp } from "../state.svelte";
  import AppIcon from "./AppIcon.svelte";
  import Badge from "./Badge.svelte";

  let { app }: { app: Card } = $props();
</script>

<button class="row" onclick={() => openApp(app.id)}>
  <AppIcon src={app.icon} kind={app.ways[0].kind} size={48} />
  <span class="text">
    <span class="name">{app.name}</span>
    <span class="summary">{app.summary}</span>
  </span>
  <span class="ways">
    {#each app.ways as way (way.method + way.label)}
      <Badge kind={way.kind} label={way.label} status={way.status} />
    {/each}
  </span>
</button>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 16px;
    width: 100%;
    padding: 12px 8px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: transparent;
    text-align: left;
    border-radius: 0;
  }
  .row:hover {
    background: var(--surface);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    font-weight: 600;
  }
  .summary {
    color: var(--muted);
    font-size: 14px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ways {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
    max-width: 45%;
  }
</style>
