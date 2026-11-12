<script lang="ts">
  import { api, message, type AppPage, type Preview } from "../api";
  import AppIcon from "../components/AppIcon.svelte";
  import Badge from "../components/Badge.svelte";
  import Confirm from "../components/Confirm.svelte";
  import Icon from "../components/Icon.svelte";
  import JobStatus from "../components/JobStatus.svelte";
  import { back, changes, jobs, startJob } from "../state.svelte";

  let { id }: { id: string } = $props();

  let page = $state<AppPage | null>(null);
  let error = $state("");
  let method = $state("");
  let shot = $state(0);

  // What the confirmation is for, while it's open.
  let asking = $state<{ action: "install" | "remove"; preview: Preview; error: string; working: boolean } | null>(null);

  async function load(appId: string) {
    try {
      page = await api.details(appId);
      error = "";
    } catch (e) {
      error = message(e);
    }
  }

  $effect(() => {
    method = "";
    shot = 0;
    page = null;
    load(id);
  });

  // Reload when an install or removal ends.
  let seen = changes.count;
  $effect(() => {
    if (changes.count !== seen) {
      seen = changes.count;
      load(id);
    }
  });

  const job = $derived(jobs[id]);
  const running = $derived(job?.status === "running");
  const choice = $derived(page?.choices.find((c) => c.method === method) ?? page?.choices[0]);

  async function begin(action: "install" | "remove") {
    if (!page || !choice) return;
    try {
      const preview = action === "install" ? await api.previewInstall(id, choice.method) : await api.previewRemove(id);
      if (action === "install" && !preview.needs_password && !preview.setup && !preview.warning) {
        await run(action, null);
      } else {
        asking = { action, preview, error: "", working: false };
      }
    } catch (e) {
      error = message(e);
    }
  }

  async function run(action: "install" | "remove", password: string | null) {
    if (!choice) return;
    if (asking) asking.working = true;
    // Before the call: its first events can arrive before it returns.
    startJob(id, `${action === "install" ? "Installing" : "Removing"} ${page!.card.name}`);
    try {
      if (action === "install") await api.install(id, choice.method, password);
      else await api.remove(id, password);
      asking = null;
    } catch (e) {
      delete jobs[id];
      if (asking) {
        asking.error = message(e);
        asking.working = false;
      } else error = message(e);
    }
  }

  async function openIt() {
    try {
      await api.open(id);
      error = "";
    } catch (e) {
      error = message(e);
    }
  }

  const site = $derived.by(() => {
    try {
      return page?.homepage ? new URL(page.homepage).host.replace(/^www\./, "") : "";
    } catch {
      return page?.homepage ?? "";
    }
  });
</script>

<div class="page">
  <button class="back" onclick={back}><Icon name="back" size={16} />Back</button>

  {#if error && !page}
    <p class="muted">{error}</p>
  {:else if page && choice}
    <div class="top">
      <div class="identity">
        <AppIcon src={page.card.icon} kind={page.card.ways[0].kind} size={112} />
        <div class="names">
          <h1>{page.card.name}</h1>
          <p class="summary">{page.card.summary}</p>
          <div class="ways">
            {#each page.card.ways as way (way.method + way.label)}
              <Badge kind={way.kind} label={way.label} status={way.status} />
            {/each}
          </div>
        </div>
      </div>

      <section class="panel" aria-labelledby="how">
        {#if running}
          <h2 id="how" class="label">Working</h2>
          <JobStatus {job} />
        {:else if page.installed}
          <h2 id="how" class="label">Installed</h2>
          <p>Installed as <strong>{page.installed}</strong>.</p>
          {#if job}<JobStatus {job} />{/if}
          <div class="row-buttons">
            <button class="btn primary big grow" onclick={openIt}>Open</button>
            <button class="btn big" onclick={() => begin("remove")}>Remove</button>
          </div>
        {:else}
          <h2 id="how" class="label">How to install</h2>
          <div class="choices" role="radiogroup" aria-labelledby="how">
            {#each page.choices as c, i (c.method)}
              <button
                role="radio"
                aria-checked={c.method === choice.method}
                class="choice"
                class:on={c.method === choice.method}
                onclick={() => (method = c.method)}
              >
                <span class="dot"></span>
                <span class="choice-text">
                  <span class="choice-label">{c.label}</span>
                  <span class="choice-sub">{c.sub}</span>
                </span>
                {#if i === 0 && page.choices.length > 1}<span class="best">Best here</span>{/if}
              </button>
            {/each}
          </div>
          {#if choice.command}
            <div class="command mono"><span class="prompt">$ </span>{choice.command}</div>
          {/if}
          {#if choice.warn}
            <p class="warn"><Icon name="warning" size={18} /><span>{choice.note}</span></p>
          {:else}
            <p class="note">{choice.note}</p>
          {/if}
          {#if choice.needs}
            <p class="note">{choice.needs} isn't installed yet. Whistle sets it up first.</p>
          {/if}
          {#if job}<JobStatus {job} />{/if}
          <button class="btn primary big" onclick={() => begin("install")}>Install</button>
        {/if}
        {#if error}<p class="note" role="alert">{error}</p>{/if}
      </section>
    </div>

    {#if page.screenshots.length}
      <section class="gallery" aria-label="Screenshots">
        <img class="shot" src={page.screenshots[shot]} alt="{page.card.name} screenshot {shot + 1}" />
        {#if page.screenshots.length > 1}
          <div class="thumbs">
            {#each page.screenshots as src, i (src)}
              <button class="thumb" class:on={i === shot} aria-label="Show screenshot {i + 1}" onclick={() => (shot = i)}>
                <img {src} alt="" loading="lazy" />
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    <section aria-labelledby="details" class="details">
      <h2 id="details">Details</h2>
      <dl>
        {#if page.license}<div><dt>License</dt><dd>{page.license.replace("LicenseRef-proprietary", "Proprietary")}</dd></div>{/if}
        {#if site}<div><dt>Website</dt><dd><a href={page.homepage} target="_blank" rel="noreferrer">{site}</a></dd></div>{/if}
        {#if page.categories.length}<div><dt>Categories</dt><dd>{page.categories.slice(0, 3).join(", ")}</dd></div>{/if}
        <div><dt>ID</dt><dd class="mono id">{page.card.id}</dd></div>
      </dl>
    </section>
  {/if}
</div>

{#if asking && page}
  <Confirm
    title={asking.action === "install" ? `Install ${page.card.name}?` : `Remove ${page.card.name}?`}
    action={asking.action === "install" ? "Install" : "Remove"}
    preview={asking.preview}
    error={asking.error}
    working={asking.working}
    onconfirm={(password) => run(asking!.action, password)}
    oncancel={() => (asking = null)}
  />
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 32px;
  }
  .back {
    align-self: flex-start;
    height: 36px;
    padding: 0 14px 0 10px;
    border: 1px solid var(--line);
    border-radius: 18px;
    background: transparent;
    color: var(--text-soft);
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
  }
  .back:hover {
    background: var(--hover);
  }
  .top {
    display: flex;
    flex-wrap: wrap;
    gap: 32px;
    align-items: flex-start;
  }
  .identity {
    flex: 999 1 380px;
    min-width: 0;
    display: flex;
    gap: 24px;
    align-items: center;
  }
  .names {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  h1 {
    font-size: 48px;
    line-height: 1.05;
    letter-spacing: -0.03em;
    overflow-wrap: anywhere;
  }
  .summary {
    color: var(--text-soft);
    font-size: 17px;
  }
  .ways {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-top: 4px;
  }
  .panel {
    flex: 1 1 340px;
    max-width: 460px;
    padding: 20px;
    border-radius: 20px;
    background: var(--raised);
    border: 1px solid var(--line-strong);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .label {
    font-family: var(--sans);
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border-radius: 14px;
    border: 1px solid #2e333d;
    background: transparent;
    text-align: left;
  }
  .choice:hover:not(:disabled) {
    background: var(--hover);
  }
  .choice.on {
    border-color: var(--text);
    background: var(--hover);
  }
  .dot {
    flex: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid #5a606b;
  }
  .choice.on .dot {
    border: 5px solid var(--text);
  }
  .choice-text {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .choice-label {
    font-weight: 600;
  }
  .choice-sub,
  .best {
    color: var(--muted);
    font-size: 13px;
  }
  .command {
    padding: 12px 14px;
    border-radius: 12px;
    background: var(--ink-deep);
    font-size: 13px;
    color: var(--text-soft);
    overflow-x: auto;
    white-space: nowrap;
  }
  .prompt {
    color: var(--faint);
  }
  .warn {
    display: flex;
    gap: 10px;
    padding: 12px 14px;
    border-radius: 12px;
    background: var(--amber-wash);
    color: var(--amber-text);
    font-size: 13px;
    line-height: 1.45;
  }
  .warn :global(svg) {
    flex: none;
    color: var(--amber);
    margin-top: 1px;
  }
  .note {
    color: var(--muted);
    font-size: 13px;
  }
  .big {
    height: 52px;
    border-radius: 26px;
    font-size: 16px;
  }
  .row-buttons {
    display: flex;
    gap: 10px;
  }
  .grow {
    flex: 1;
  }
  .gallery {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .shot {
    width: 100%;
    max-height: 640px;
    object-fit: contain;
    border-radius: 16px;
    background: #000;
    display: block;
  }
  .thumbs {
    display: flex;
    gap: 10px;
    overflow-x: auto;
    padding-bottom: 4px;
  }
  .thumb {
    flex: none;
    padding: 0;
    border-radius: 10px;
    border: 2px solid transparent;
    background: none;
    opacity: 0.6;
  }
  .thumb.on {
    border-color: var(--text);
    opacity: 1;
  }
  .thumb img {
    width: 160px;
    height: 90px;
    object-fit: cover;
    display: block;
    border-radius: 8px;
  }
  .details {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .details h2 {
    font-size: 24px;
  }
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 1px;
    background: var(--line);
    border-radius: 14px;
    overflow: hidden;
  }
  dl div {
    background: var(--ink);
    padding: 14px 18px;
  }
  dt {
    color: var(--muted);
    font-size: 13px;
  }
  dd {
    margin: 0;
    font-weight: 500;
  }
  dd a {
    color: var(--text);
  }
  .id {
    font-size: 13px;
    overflow-wrap: anywhere;
  }
</style>
