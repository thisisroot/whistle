// Where the window is, and what's running in the background.

import { listen } from "@tauri-apps/api/event";
import { api, message, type JobEvent, type Overview } from "./api";

export type Page = { name: "discover" } | { name: "search" } | { name: "installed" } | { name: "app"; id: string };

export const nav = $state({
  page: { name: "discover" } as Page,
  history: [] as Page[],
  query: "",
  filter: "all",
  category: "",
});

export function go(page: Page) {
  if (nav.page.name === "app" || page.name === "app") nav.history.push(nav.page);
  else nav.history = [];
  nav.page = page;
}

export function openApp(id: string) {
  go({ name: "app", id });
}

export function back() {
  nav.page = nav.history.pop() ?? { name: "discover" };
}

export const catalog = $state({
  overview: null as Overview | null,
  error: "",
  loading: false,
});

export async function loadOverview(refresh = false) {
  catalog.loading = true;
  catalog.error = "";
  try {
    catalog.overview = refresh ? await api.refresh() : await api.overview();
  } catch (e) {
    catalog.error = message(e);
  } finally {
    catalog.loading = false;
  }
}

export interface Job {
  status: "running" | "done" | "failed";
  /** What it's doing: "Installing GIMP". */
  label: string;
  /** The latest step, note or line of output. */
  line: string;
  log: string[];
  /** How it ended. */
  result: string;
}

export const jobs = $state<Record<string, Job>>({});

/** Bumped when a job ends, so pages showing installed apps reload. */
export const changes = $state({ count: 0 });

export function startJob(id: string, label: string) {
  jobs[id] = { status: "running", label, line: "Starting", log: [], result: "" };
}

const MAX_LOG = 400;

export async function listenForJobs() {
  await listen<JobEvent>("job", ({ payload }) => {
    const job = (jobs[payload.job] ??= { status: "running", label: "Working", line: "", log: [], result: "" });
    switch (payload.kind) {
      case "step":
        job.line = payload.text;
        job.log.push("$ " + payload.text);
        break;
      case "note":
      case "log":
        if (payload.text.trim()) job.line = payload.text;
        job.log.push(payload.text);
        break;
      case "done":
      case "failed":
        job.status = payload.kind;
        job.result = payload.text;
        changes.count++;
        break;
    }
    if (job.log.length > MAX_LOG) job.log.splice(0, job.log.length - MAX_LOG);
  });
}
