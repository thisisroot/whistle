<script lang="ts">
  import Icon from "./lib/components/Icon.svelte";
  import AppPage from "./lib/pages/AppPage.svelte";
  import Discover from "./lib/pages/Discover.svelte";
  import Installed from "./lib/pages/Installed.svelte";
  import Search from "./lib/pages/Search.svelte";
  import { catalog, go, jobs, listenForJobs, loadOverview, nav } from "./lib/state.svelte";

  let main: HTMLElement;

  $effect(() => {
    listenForJobs();
    loadOverview();
  });

  // Each page starts at the top.
  $effect(() => {
    void nav.page;
    main?.scrollTo({ top: 0 });
  });

  const section = $derived(nav.page.name === "app" ? (nav.history.at(-1)?.name ?? "discover") : nav.page.name);
  const running = $derived(Object.values(jobs).filter((j) => j.status === "running").length);

  function typed(e: Event) {
    nav.query = (e.target as HTMLInputElement).value;
    if (nav.page.name !== "search") go({ name: "search" });
  }

  function home() {
    nav.query = "";
    nav.category = "";
    nav.filter = "all";
    go({ name: "discover" });
  }
</script>

<div class="shell">
  <nav aria-label="Main">
    <div class="brand">
      <svg width="36" height="36" viewBox="0 0 128 128" aria-hidden="true">
        <rect width="128" height="128" rx="30" fill="#0F1115" />
        <rect x="1" y="1" width="126" height="126" rx="29" fill="none" stroke="#FFFFFF" stroke-opacity="0.16" stroke-width="2" />
        <path fill="#FFFFFF" fill-rule="evenodd" d="M72 50H24a6 6 0 0 0-6 6v8a6 6 0 0 0 6 6h21.2A28 28 0 1 0 72 50Z M72 68a10 10 0 1 0 0 20a10 10 0 1 0 0-20Z" />
        <g fill="none" stroke="#FFFFFF" stroke-width="6" stroke-linecap="round">
          <path d="M86.5 37.9A20 20 0 0 1 97.3 50.8" /><path d="M91.5 27A32 32 0 0 1 108.9 47.7" />
        </g>
      </svg>
      <span>whistle</span>
    </div>
    <button class:on={section === "discover"} aria-current={section === "discover" ? "page" : undefined} onclick={home}>
      <Icon name="discover" />Discover
    </button>
    <button class:on={section === "search"} aria-current={section === "search" ? "page" : undefined} onclick={() => go({ name: "search" })}>
      <Icon name="search" />Search
    </button>
    <button class:on={section === "installed"} aria-current={section === "installed" ? "page" : undefined} onclick={() => go({ name: "installed" })}>
      <Icon name="installed" /><span class="grow">Installed</span>
      {#if running}<span class="count" aria-label="{running} running">{running}</span>{/if}
    </button>
    <div class="grow"></div>
    {#if catalog.overview}
      <div class="host">
        <span class="eyebrow">This computer</span>
        <span class="host-name">{catalog.overview.host.name} · {catalog.overview.host.arch}</span>
        <span class="mono tools">{catalog.overview.host.tools.join(" · ")}</span>
        <button class="refresh" onclick={() => loadOverview(true)} disabled={catalog.loading}>
          <Icon name="refresh" size={14} />{catalog.loading ? "Updating catalog…" : "Update catalog"}
        </button>
      </div>
    {/if}
  </nav>

  <main bind:this={main}>
    <label class="searchbar">
      <Icon name="search" size={18} />
      <span class="visually-hidden">Search apps</span>
      <input
        type="search"
        placeholder={catalog.overview
          ? `Search ${catalog.overview.total.toLocaleString("en-US")} apps`
          : "Search apps"}
        value={nav.query}
        oninput={typed}
      />
    </label>

    {#if !catalog.overview}
      <div class="loading">
        {#if catalog.error}
          <h1>Couldn't load the catalog</h1>
          <p class="muted">{catalog.error}</p>
          <button class="btn primary" onclick={() => loadOverview()}>Try again</button>
        {:else}
          <h1>Getting the catalog</h1>
          <p class="muted">The first time, Whistle downloads the list of apps and works out which ones this computer can run.</p>
        {/if}
      </div>
    {:else if nav.page.name === "discover"}
      <Discover overview={catalog.overview} />
    {:else if nav.page.name === "search"}
      <Search />
    {:else if nav.page.name === "installed"}
      <Installed />
    {:else if nav.page.name === "app"}
      {#key nav.page.id}<AppPage id={nav.page.id} />{/key}
    {/if}
  </main>
</div>

<style>
  .shell {
    display: flex;
    height: 100vh;
  }
  nav {
    flex: none;
    width: 240px;
    padding: 24px 16px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-right: 1px solid var(--line);
    background: var(--ink-deep);
    overflow-y: auto;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 8px 24px;
    font-family: var(--display);
    font-weight: 700;
    font-size: 22px;
    letter-spacing: -0.02em;
  }
  nav > button {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 44px;
    padding: 0 12px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    color: var(--muted);
    font-weight: 500;
    text-align: left;
  }
  nav > button:hover {
    color: var(--text);
    background: var(--surface);
  }
  nav > button.on {
    color: #ffffff;
    background: var(--hover);
  }
  .grow {
    flex: 1;
  }
  .count {
    min-width: 22px;
    padding: 0 6px;
    border-radius: 11px;
    background: var(--amber);
    color: var(--amber-ink);
    font-size: 12px;
    font-weight: 600;
    line-height: 22px;
    text-align: center;
  }
  .host {
    padding: 14px;
    border: 1px solid var(--line);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
  }
  .eyebrow {
    color: var(--muted);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .host-name {
    font-weight: 600;
  }
  .tools {
    color: var(--muted);
    font-size: 12px;
  }
  .refresh {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 12px;
  }
  .refresh:hover:not(:disabled) {
    color: var(--text);
  }
  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 24px clamp(16px, 4vw, 48px) 64px;
    display: flex;
    flex-direction: column;
    gap: 32px;
  }
  .searchbar {
    flex: none;
    max-width: 640px;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 48px;
    padding: 0 16px;
    border-radius: 24px;
    background: var(--raised);
    border: 1px solid var(--line-strong);
    color: var(--muted);
  }
  .searchbar:focus-within {
    border-color: var(--text-soft);
  }
  .searchbar input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .loading {
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: flex-start;
    max-width: 52ch;
    padding-top: 48px;
  }
  .loading h1 {
    font-size: 36px;
  }
  @media (max-width: 760px) {
    .shell {
      flex-direction: column;
    }
    nav {
      width: auto;
      flex-direction: row;
      flex-wrap: wrap;
      padding: 12px 16px;
      border-right: 0;
      border-bottom: 1px solid var(--line);
    }
    .brand {
      padding: 0 8px 0 0;
    }
    nav > .grow,
    .host {
      display: none;
    }
  }
</style>
