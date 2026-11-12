<script lang="ts">
  import type { Kind } from "../api";

  // How an app runs: told apart by fill, not only by colour.
  let { kind, label, status = null }: { kind: Kind; label: string; status?: string | null } = $props();
  const look = $derived(
    kind === "windows" ? (status === "untested" || status === "draft" ? "untested" : "windows") : kind,
  );
</script>

<span class="badge {look}">{label}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border-radius: 11px;
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .native {
    background: var(--text);
    color: var(--ink);
  }
  .flatpak,
  .appimage {
    border: 1px solid #5a606b;
  }
  .windows {
    background: var(--tile);
    color: #dce1ea;
  }
  .untested {
    background: var(--amber);
    color: var(--amber-ink);
  }
</style>
