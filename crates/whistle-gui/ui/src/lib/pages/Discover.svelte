<script lang="ts">
  import type { Overview } from "../api";
  import AppCard from "../components/AppCard.svelte";
  import AppIcon from "../components/AppIcon.svelte";
  import Badge from "../components/Badge.svelte";
  import { go, nav, openApp } from "../state.svelte";

  let { overview }: { overview: Overview } = $props();
  const featured = $derived(overview.featured);
  const n = (count: number) => count.toLocaleString("en-US");

  function browse(category: string) {
    nav.category = category;
    nav.query = "";
    nav.filter = "all";
    go({ name: "search" });
  }

  function windowsApps() {
    nav.category = "";
    nav.query = "";
    nav.filter = "windows";
    go({ name: "search" });
  }
</script>

<div class="discover">
  {#if featured}
    <section class="hero" aria-labelledby="featured">
      <div class="hero-text">
        <span class="eyebrow mono">Featured</span>
        <div class="title">
          <AppIcon src={featured.card.icon} kind={featured.card.ways[0].kind} size={64} />
          <h1 id="featured">{featured.card.name}</h1>
        </div>
        <p class="summary">{featured.card.summary}</p>
        <div class="ways">
          {#each featured.card.ways as way (way.method + way.label)}
            <Badge kind={way.kind} label={way.label} status={way.status} />
          {/each}
        </div>
        <div><button class="btn primary" onclick={() => openApp(featured.card.id)}>View {featured.card.name}</button></div>
      </div>
      <img class="shot" src={featured.screenshot} alt="{featured.card.name} screenshot" />
    </section>
  {/if}

  <section class="stats" aria-label="What you can install here">
    <div><strong class="mono">{n(overview.total)}</strong><span>apps you can install here</span></div>
    <div><strong class="mono">{n(overview.native)}</strong><span>from your distro</span></div>
    <div><strong class="mono">{n(overview.flatpak)}</strong><span>on Flathub</span></div>
    <div><strong class="mono">{n(overview.windows)}</strong><span>for Windows, through Wine</span></div>
  </section>

  {#if overview.popular.length}
    <section aria-labelledby="popular">
      <h2 id="popular">Popular</h2>
      <div class="grid">
        {#each overview.popular as app (app.id)}<AppCard {app} />{/each}
      </div>
    </section>
  {/if}

  {#if overview.windows}
    <section aria-labelledby="windows">
      <h2 id="windows">From Windows</h2>
      <div class="grid">
        {#each overview.windows_verified as app (app.id)}<AppCard {app} />{/each}
        <button class="more" onclick={windowsApps}>
          <strong>{n(overview.windows)} Windows apps</strong>
          <span>
            Most were generated from winget and are marked <Badge kind="windows" label="untested" status="untested" />
            until someone tries them. Whistle starts each one after installing to check that it runs.
          </span>
        </button>
      </div>
    </section>
  {/if}

  <section aria-labelledby="cats">
    <h2 id="cats">Categories</h2>
    <div class="cats">
      {#each overview.categories.filter((c) => c.count) as category (category.id)}
        <button onclick={() => browse(category.id)}>
          <span class="cat">{category.label}</span>
          <span class="count mono">{n(category.count)} apps</span>
        </button>
      {/each}
    </div>
  </section>
</div>

<style>
  .discover {
    display: flex;
    flex-direction: column;
    gap: 48px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  h2 {
    font-size: 28px;
  }
  .hero {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 0;
    border-radius: 20px;
    overflow: hidden;
    background: var(--raised);
    border: 1px solid var(--line);
  }
  .hero-text {
    flex: 1 1 300px;
    padding: 40px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 16px;
  }
  .eyebrow {
    color: var(--amber);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .title h1 {
    font-size: 48px;
    line-height: 1;
    letter-spacing: -0.03em;
  }
  .summary {
    color: var(--text-soft);
    font-size: 17px;
    max-width: 36ch;
  }
  .ways {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .shot {
    flex: 2 1 420px;
    min-width: 0;
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    display: block;
    background: #000;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 1px;
    background: var(--line);
    border-radius: 16px;
    overflow: hidden;
  }
  .stats div {
    background: var(--ink);
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
  }
  .stats strong {
    font-size: 32px;
    font-weight: 600;
  }
  .stats span {
    color: var(--muted);
    font-size: 14px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .more {
    grid-column: span 2;
    padding: 20px 24px;
    border-radius: 16px;
    border: 1px dashed var(--line-strong);
    background: transparent;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 8px;
    justify-content: center;
  }
  .more:hover {
    background: var(--surface);
  }
  .more span {
    color: var(--muted);
    font-size: 14px;
  }
  @media (max-width: 720px) {
    .more {
      grid-column: auto;
    }
  }
  .cats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 12px;
  }
  .cats button {
    height: 96px;
    padding: 16px 18px;
    border: 1px solid var(--line);
    border-radius: 16px;
    background: var(--surface);
    text-align: left;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }
  .cats button:hover {
    background: var(--hover);
    border-color: var(--line-strong);
  }
  .cat {
    font-family: var(--display);
    font-size: 20px;
    font-weight: 500;
  }
  .count {
    color: var(--muted);
    font-size: 13px;
  }
</style>
