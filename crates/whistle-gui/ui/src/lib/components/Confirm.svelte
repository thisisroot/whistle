<script lang="ts">
  import type { Preview } from "../api";
  import Icon from "./Icon.svelte";

  // Shows what's about to run, and asks for the password when sudo needs it.
  let {
    title,
    action,
    preview,
    error = "",
    working = false,
    onconfirm,
    oncancel,
  }: {
    title: string;
    action: string;
    preview: Preview;
    error?: string;
    working?: boolean;
    onconfirm: (password: string | null) => void;
    oncancel: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let password = $state("");

  $effect(() => {
    dialog.showModal();
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    onconfirm(preview.needs_password ? password : null);
  }
</script>

<dialog bind:this={dialog} oncancel={(e) => (e.preventDefault(), oncancel())} aria-labelledby="confirm-title">
  <form onsubmit={submit}>
    <h2 id="confirm-title">{title}</h2>

    {#if preview.setup}
      <p>{preview.setup} isn't on this computer yet. Whistle installs it first.</p>
    {/if}

    {#if preview.warning}
      <p class="warn"><Icon name="warning" size={18} /><span>{preview.warning}</span></p>
    {/if}

    {#if preview.commands.length}
      <div class="commands mono">
        {#each preview.commands as command}
          <div><span class="prompt">$</span> {command}</div>
        {/each}
      </div>
    {/if}

    {#if preview.needs_password}
      <label>
        <span>Your password, for sudo</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input type="password" bind:value={password} autocomplete="current-password" autofocus required />
      </label>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="actions">
      <button type="button" class="btn" onclick={oncancel}>Cancel</button>
      <button type="submit" class="btn primary" disabled={working}>{working ? "Checking…" : action}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: min(560px, calc(100vw - 32px));
    padding: 0;
    border: 1px solid var(--line-strong);
    border-radius: 20px;
    background: var(--raised);
    color: var(--text);
  }
  dialog::backdrop {
    background: rgb(0 0 0 / 0.6);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 24px;
  }
  h2 {
    font-size: 24px;
  }
  .warn {
    display: flex;
    gap: 10px;
    padding: 12px 14px;
    border-radius: 12px;
    background: var(--amber-wash);
    color: var(--amber-text);
    font-size: 14px;
  }
  .warn :global(svg) {
    flex: none;
    color: var(--amber);
    margin-top: 2px;
  }
  .commands {
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
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 14px;
    color: var(--muted);
  }
  input {
    height: 44px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--line-strong);
    background: var(--ink);
    color: var(--text);
  }
  .error {
    color: var(--amber-text);
    font-size: 14px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }
</style>
