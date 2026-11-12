<script lang="ts">
  import { api, message, type InstalledList } from "../api";
  import AppIcon from "../components/AppIcon.svelte";
  import Badge from "../components/Badge.svelte";
  import Confirm from "../components/Confirm.svelte";
  import JobStatus from "../components/JobStatus.svelte";
  import { changes, jobs, openApp, startJob } from "../state.svelte";

  let list = $state<InstalledList | null>(null);
  let error = $state("");
  let asking = $state<{ error: string; working: boolean } | null>(null);

  async function load() {
    try {
      list = await api.installed();
      error = "";
    } catch (e) {
      error = message(e);
    }
  }

  $effect(() => {
    void changes.count;
    load();
  });

  const job = $derived(jobs["upgrade"]);
  const updates = $derived(list?.apps.filter((a) => a.update) ?? []);

  async function upgrade(password: string | null) {
    if (asking) asking.working = true;
    startJob("upgrade", "Updating");
    try {
      await api.upgradeAll(password);
      asking = null;
    } catch (e) {
      delete jobs["upgrade"];
      if (asking) {
        asking.error = message(e);
        asking.working = false;
      } else error = message(e);
    }
  }

  async function openIt(id: string) {
    try {
      await api.open(id);
      error = "";
    } catch (e) {
      error = message(e);
    }
  }

  function begin() {
    if (list?.preview.needs_password) asking = { error: "", working: false };
    else upgrade(null);
  }
</script>

<div class="installed">
  <div class="head">
    <h1>Installed</h1>
    {#if list?.apps.length}
      <button class="btn primary" disabled={job?.status === "running"} onclick={begin}>Update everything</button>
    {/if}
  </div>

  {#if job}<JobStatus {job} />{/if}

  {#if error}
    <p class="muted">{error}</p>
  {:else if list}
    {#if list.managers.length}
      <p class="muted">
        Updating refreshes apps from {list.managers.join(" and ")}{updates.length ? `, and reinstalls ${updates.length} Windows app${updates.length === 1 ? "" : "s"} with a newer recipe` : ""}.
      </p>
    {/if}
    {#if list.apps.length === 0}
      <p class="muted">Nothing installed with Whistle yet.</p>
    {:else}
      <div class="list">
        {#each list.apps as app (app.id)}
          <div class="row">
            <button class="about" onclick={() => openApp(app.id)}>
              <AppIcon src={app.icon} kind={app.kind} size={48} />
              <span class="text">
                <span class="name">{app.name}</span>
                <span class="summary">{app.update ? `Update: ${app.update}` : app.summary}</span>
              </span>
              <Badge kind={app.kind} label={app.how} />
            </button>
            {#if jobs[app.id]?.status === "running"}
              <span class="muted busy">Working…</span>
            {:else}
              <button class="btn" onclick={() => openIt(app.id)}>Open</button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

{#if asking && list}
  <Confirm
    title="Update everything?"
    action="Update"
    preview={list.preview}
    error={asking.error}
    working={asking.working}
    onconfirm={upgrade}
    oncancel={() => (asking = null)}
  />
{/if}

<style>
  .installed {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px;
  }
  h1 {
    flex: 1;
    font-size: 36px;
  }
  .list {
    border-top: 1px solid var(--line);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-right: 8px;
    border-bottom: 1px solid var(--line);
  }
  .about {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 8px;
    border: 0;
    border-radius: 10px;
    background: transparent;
    text-align: left;
  }
  .about:hover {
    background: var(--surface);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
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
  .busy {
    font-size: 13px;
  }
</style>
