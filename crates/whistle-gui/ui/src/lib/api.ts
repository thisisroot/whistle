// The commands in `src/main.rs`, typed.

import { invoke } from "@tauri-apps/api/core";

export type Kind = "native" | "flatpak" | "appimage" | "windows";

export interface Way {
  method: string;
  kind: Kind;
  label: string;
  status: string | null;
  ready: boolean;
}

export interface Card {
  id: string;
  name: string;
  summary: string;
  icon: string;
  ways: Way[];
}

export interface Overview {
  host: { name: string; arch: string; tools: string[]; in_wsl: boolean };
  generated_at: string;
  total: number;
  native: number;
  flatpak: number;
  windows: number;
  featured: { card: Card; screenshot: string } | null;
  popular: Card[];
  windows_verified: Card[];
  categories: { id: string; label: string; count: number }[];
}

export interface Results {
  total: number;
  apps: Card[];
}

export interface Choice extends Way {
  sub: string;
  command: string | null;
  note: string;
  warn: boolean;
  needs: string | null;
}

export interface AppPage {
  card: Card;
  homepage: string;
  license: string;
  categories: string[];
  screenshots: string[];
  choices: Choice[];
  installed: string | null;
}

export interface Preview {
  commands: string[];
  needs_password: boolean;
  setup: string | null;
  warning: string | null;
}

export interface InstalledApp {
  id: string;
  name: string;
  summary: string;
  icon: string;
  how: string;
  kind: Kind;
  update: string | null;
}

export interface InstalledList {
  apps: InstalledApp[];
  managers: string[];
  preview: Preview;
}

export interface JobEvent {
  job: string;
  kind: "step" | "note" | "log" | "done" | "failed";
  text: string;
}

export const api = {
  overview: () => invoke<Overview>("overview"),
  refresh: () => invoke<Overview>("refresh"),
  search: (query: string, filter: string, category: string) =>
    invoke<Results>("search", { query, filter, category }),
  details: (id: string) => invoke<AppPage>("details", { id }),
  previewInstall: (id: string, method: string) =>
    invoke<Preview>("preview_install", { id, method }),
  install: (id: string, method: string, password: string | null) =>
    invoke<void>("install", { id, method, password }),
  previewRemove: (id: string) => invoke<Preview>("preview_remove", { id }),
  remove: (id: string, password: string | null) => invoke<void>("remove", { id, password }),
  open: (id: string) => invoke<void>("open", { id }),
  installed: () => invoke<InstalledList>("installed"),
  upgradeAll: (password: string | null) => invoke<void>("upgrade_all", { password }),
};

/** A command's error message. */
export function message(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
