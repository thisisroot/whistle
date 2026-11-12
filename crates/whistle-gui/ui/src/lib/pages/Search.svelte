<script lang="ts">
  import { api, message, type Results } from "../api";
  import AppRow from "../components/AppRow.svelte";
  import { catalog, nav } from "../state.svelte";

  const FILTERS = [
    ["all", "Everything"],
    ["native", "From your distro"],
    ["flatpak", "Flatpak"],
    ["windows", "Windows apps"],
  ] as const;

  let results = $state<Results | null>(null);
  let error = $state("");
  let latest = 0;

  // Search as the user types, a moment after they stop.
  $effect(() => {
    const { query, filter, category } = nav;
    const ticket = ++latest;
    const timer = setTimeout(async () => {
      try {
        const found = await api.search(query, filter, category);
        if (ticket === latest) {
          results = found;
          error = "";
        }
      } catch (e) {
        if (ticket === latest) error = message(e);
      }
    }, 120);
    return () => clearTimeout(timer);
  });

  const categoryLabel = $derived(
    catalog.overview?.categories.find((c) => c.id === nav.category)?.label ?? nav.category,
  );
  const title = $derived(nav.query.trim() ? `Results for “${nav.query.trim()}”` : categoryLabel || "All apps");
  const n = (count: number) => count.toLocaleString("en-US");
</script>

<div class="search">
  <div class="head">
    <h1>{title}</h1>
    {#if results}<span class="muted">{results.total === 1 ? "1 app" : `${n(results.total)} apps`}</span>{/if}
    {#if nav.category}
      <button class="chip" aria-label="Show every category" onclick={() => (nav.category = "")}>{categoryLabel} ✕</button>
    {/if}
  </div>

  <div class="filters" role="group" aria-label="How it runs">
    {#each FILTERS as [key, label] (key)}
      <button class="chip" class:on={nav.filter === key} aria-pressed={nav.filter === key} onclick={() => (nav.filter = key)}>
        {label}
      </button>
    {/each}
  </div>

  {#if error}
    <p class="muted">{error}</p>
  {:else if results}
    <div class="list">
      {#each results.apps as app (app.id)}<AppRow {app} />{/each}
    </div>
    {#if results.total === 0}
      <p class="muted empty">Nothing you can install here matches. Try fewer words, or another filter.</p>
    {:else if results.total > results.apps.length}
      <p class="muted empty">Showing the best {results.apps.length}. Type more to narrow it down.</p>
    {/if}
  {/if}
</div>

<style>
  .search {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 12px;
  }
  h1 {
    font-size: 36px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .chip {
    height: 36px;
    padding: 0 16px;
    border-radius: 18px;
    border: 1px solid var(--line-strong);
    background: transparent;
    font-size: 14px;
  }
  .chip:hover {
    background: var(--hover);
  }
  .chip.on {
    border-color: var(--text);
    background: var(--text);
    color: var(--ink);
    font-weight: 600;
  }
  .list {
    border-top: 1px solid var(--line);
  }
  .empty {
    padding: 16px 8px;
  }
</style>
