<script lang="ts">
  import type { Job } from "../state.svelte";

  // A running or finished job: what it's doing, and its output on request.
  let { job }: { job: Job } = $props();
  let showLog = $state(false);
  let logBox: HTMLElement | undefined = $state();

  $effect(() => {
    void job.log.length;
    if (logBox) logBox.scrollTop = logBox.scrollHeight;
  });
</script>

<div class="job" role="status">
  {#if job.status === "running"}
    <div class="head"><span>{job.label}</span></div>
    <div class="bar"><div class="fill"></div></div>
    <div class="line mono">{job.line}</div>
  {:else}
    <div class="head {job.status}">{job.result}</div>
  {/if}
  {#if job.log.length}
    <button class="toggle" onclick={() => (showLog = !showLog)} aria-expanded={showLog}>
      {showLog ? "Hide details" : "Show details"}
    </button>
    {#if showLog}
      <pre class="log mono" bind:this={logBox}>{job.log.join("\n")}</pre>
    {/if}
  {/if}
</div>

<style>
  .job {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .head {
    font-size: 14px;
    font-weight: 500;
  }
  .head.failed {
    color: var(--amber-text);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--line-strong);
    overflow: hidden;
  }
  .fill {
    width: 30%;
    height: 100%;
    border-radius: 3px;
    background: var(--text);
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
  }
  .line {
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .toggle {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 13px;
    text-decoration: underline;
  }
  .log {
    margin: 0;
    max-height: 220px;
    overflow: auto;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--ink-deep);
    color: var(--text-soft);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
