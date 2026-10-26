// The settings window (work item 04-02).
//
// A form over `settings.toml`: the same file you can edit by hand, and the
// same path into the app either way. Saving writes the file; the core's
// watcher notices and publishes the settings back, which is what actually
// applies them. Nothing here changes the app directly — if it did, the two
// ways of changing a setting could disagree.
//
// The per-plugin parts are generated from the plugin registry, so a plugin
// installed tomorrow gets its own rows without a line of code here.

import { invoke, subscribe } from "./ipc";
import { pick, prompt } from "./picker";
import type { PluginsStatus, ServiceOverride, Settings } from "./protocol";
import { toast } from "./toast";

/** What the window applies as soon as it hears settings changed. */
export interface SettingsHost {
  /** Font, tab size, and anything else the editor owns. */
  applyEditor(settings: Settings): void;
  /** Close every tab, because the library moved out from under them. */
  libraryMoved(root: string): Promise<void>;
}

/** The settings as the window last heard them. */
let current: Settings | null = null;

/** Called whenever the core publishes new settings. */
const listeners = new Set<(settings: Settings) => void>();

/** Start listening for settings, and apply the ones in force now. */
export async function watchSettings(host: SettingsHost): Promise<Settings> {
  subscribe(({ event }) => {
    if (event.type !== "settings") return;
    // The event is the settings, with a `type` alongside them.
    const { type: _type, ...settings } = event;
    current = settings;
    host.applyEditor(settings);
    for (const listener of listeners) listener(settings);
  });
  current = await invoke<Settings>("get_settings");
  host.applyEditor(current);
  return current;
}

/** The settings in force, once `watchSettings` has run. */
export function settings(): Settings | null {
  return current;
}

/** Whether a run empties the output pane first. */
export function clearOutputOnRun(): boolean {
  return current?.run.clear_output ?? true;
}

let open: HTMLElement | null = null;

export function settingsOpen(): boolean {
  return open !== null;
}

/**
 * Show the settings form. Opening it again brings the one that's up forward.
 * `at` names a row to start on: `library` is the library folder's, for the
 * library panel's "Change folder" (work item 04-12).
 */
export async function openSettings(host: SettingsHost, at?: "library"): Promise<void> {
  const start = () =>
    (at === "library" ? open?.querySelector<HTMLElement>(".settings-library-change") : null) ??
    open?.querySelector<HTMLElement>("input, select, button");
  if (open) {
    start()?.focus();
    return;
  }
  const [settings, plugins] = await Promise.all([
    invoke<Settings>("get_settings"),
    invoke<PluginsStatus>("list_plugins"),
  ]);
  const draft: Settings = structuredClone(settings);

  const backdrop = document.createElement("div");
  backdrop.className = "picker-backdrop settings-backdrop";
  const dialog = document.createElement("div");
  dialog.className = "settings";
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-label", "Settings");

  const title = document.createElement("div");
  title.className = "settings-title";
  title.textContent = "Settings";
  const body = document.createElement("div");
  body.className = "settings-body";
  const actions = document.createElement("div");
  actions.className = "settings-actions";
  dialog.append(title, body, actions);
  backdrop.append(dialog);

  // ---- the form ------------------------------------------------------------

  body.append(
    section("Appearance", [
      choice(
        "Theme",
        draft.theme,
        [
          ["system", "Follow the system"],
          ["light", "Light"],
          ["dark", "Dark"],
        ],
        (value) => (draft.theme = value),
      ),
    ]),
    section("Editor", [
      number("Font size", draft.editor.font_size, 8, 40, (n) => (draft.editor.font_size = n)),
      text(
        "Font family",
        draft.editor.font_family ?? "",
        "the platform's own",
        (value) => (draft.editor.font_family = value || null),
      ),
      number("Tab size", draft.editor.tab_size, 1, 16, (n) => (draft.editor.tab_size = n)),
    ]),
    section("Running", [
      toggle("Clear output when a run starts", draft.run.clear_output, (on) => {
        draft.run.clear_output = on;
      }),
    ]),
    section("Library", [
      libraryRow(draft, host),
      choice(
        "Panel side",
        draft.library.side,
        [
          ["left", "Left"],
          ["right", "Right"],
        ],
        (value) => (draft.library.side = value),
      ),
    ]),
  );

  const servers = plugins.plugins.filter((p) => p.lsp);
  if (servers.length) {
    body.append(
      section(
        "Language servers",
        servers.map((plugin) =>
          serviceRow(
            plugin.name,
            draft.lsp.prewarm.includes(plugin.id),
            (on) => {
              draft.lsp.prewarm = on
                ? [...draft.lsp.prewarm, plugin.id]
                : draft.lsp.prewarm.filter((id) => id !== plugin.id);
            },
            (draft.lsp[plugin.id] as ServiceOverride | undefined)?.idle_shutdown ?? "",
            (value) => setIdle(draft.lsp as Record<string, unknown>, plugin.id, value),
          ),
        ),
      ),
    );
  }

  const helpers = plugins.plugins.filter((p) => p.helper);
  if (helpers.length) {
    body.append(
      section(
        "Warm runners",
        helpers.map((plugin) =>
          text(
            plugin.name,
            draft.helpers[plugin.id]?.idle_shutdown ?? "",
            `the ${plugin.name} plugin's own`,
            (value) => setIdle(draft.helpers as Record<string, unknown>, plugin.id, value),
            "How long it may sit idle: never, or 30m",
          ),
        ),
      ),
    );
  }

  const withPrograms = plugins.plugins.filter((p) => p.programs.length > 0);
  if (withPrograms.length) {
    body.append(
      section(
        "Toolchains",
        withPrograms.flatMap((plugin) =>
          plugin.programs.map((program) =>
            text(
              `${plugin.name} · ${program}`,
              draft.toolchains[plugin.id]?.[program] ?? "",
              "found on PATH",
              (value) => {
                const programs = draft.toolchains[plugin.id] ?? {};
                if (value) programs[program] = value;
                else delete programs[program];
                if (Object.keys(programs).length) draft.toolchains[plugin.id] = programs;
                else delete draft.toolchains[plugin.id];
              },
              "A full path to the program to use instead of the one on PATH",
            ),
          ),
        ),
      ),
    );
  }

  // ---- the buttons ---------------------------------------------------------

  const close = () => {
    backdrop.remove();
    open = null;
  };

  actions.append(
    button("Open settings file", () => void invoke("open_settings_file").catch(fail)),
    button("Reset to defaults", () => void reset(close)),
    spacer(),
    button("Cancel", close),
    button("Save", () => void save(draft, close), "primary"),
  );

  backdrop.addEventListener("mousedown", (e) => {
    if (e.target === backdrop) close();
  });
  dialog.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  });

  document.body.append(backdrop);
  open = dialog;
  const first = at === "library" ? start() : dialog.querySelector<HTMLElement>("input");
  first?.scrollIntoView({ block: "nearest" });
  first?.focus();
}

async function save(draft: Settings, close: () => void): Promise<void> {
  // The library folder has its own path: it asks first and reopens the
  // library, so it isn't saved with the rest.
  const settings: Settings = { ...draft, library: { ...draft.library } };
  await invoke("save_settings", { settings }).catch(fail);
  close();
}

async function reset(close: () => void): Promise<void> {
  const answer = await pick("Put every setting back to its default?", [
    { id: "no", label: "Cancel" },
    { id: "yes", label: "Reset to defaults", detail: "Your settings file is rewritten" },
  ]);
  if (answer?.id !== "yes") return;
  await invoke("reset_settings").catch(fail);
  close();
}

function fail(e: { message?: string }): void {
  toast("error", e.message ?? "That didn't work.");
}

/** The library folder, with the confirmation its move deserves. */
function libraryRow(draft: Settings, host: SettingsHost): HTMLElement {
  const row = document.createElement("div");
  row.className = "settings-row";
  const label = document.createElement("span");
  label.className = "settings-label";
  label.textContent = "Folder";
  const value = document.createElement("span");
  value.className = "settings-value settings-path";
  value.textContent = draft.library.root ?? "your home folder";
  const change = button("Change…", () => void move());
  change.classList.add("settings-library-change");
  row.append(label, value, change);

  async function move(): Promise<void> {
    const answer = await prompt("Move the library", [
      {
        label: "Folder",
        value: draft.library.root ?? "",
        placeholder: "leave empty for the default",
        optional: true,
      },
    ]);
    if (answer === null) return;
    const root = (answer[0] ?? "").trim();
    const confirmed = await pick(`Use ${root || "the default folder"} from now on?`, [
      { id: "no", label: "Cancel" },
      {
        id: "yes",
        label: "Move the library",
        detail: "Open tabs close; playgrounds already there stay where they are",
      },
    ]);
    if (confirmed?.id !== "yes") return;
    const now = await invoke<string>("change_library_root", { root: root || null }).catch(
      (e: { message?: string }) => {
        fail(e);
        return null;
      },
    );
    if (now === null) return;
    draft.library.root = root || null;
    value.textContent = root || "your home folder";
    await host.libraryMoved(now);
    toast("info", `The library is now ${now}.`);
  }

  return row;
}

// ---- the small pieces ------------------------------------------------------

function section(title: string, rows: HTMLElement[]): HTMLElement {
  const box = document.createElement("section");
  box.className = "settings-section";
  const heading = document.createElement("h2");
  heading.className = "settings-heading";
  heading.textContent = title;
  box.append(heading, ...rows);
  return box;
}

function row(label: string, control: HTMLElement, hint?: string): HTMLElement {
  const row = document.createElement("label");
  row.className = "settings-row";
  const caption = document.createElement("span");
  caption.className = "settings-label";
  caption.textContent = label;
  row.append(caption, control);
  if (hint) {
    const note = document.createElement("span");
    note.className = "settings-hint";
    note.textContent = hint;
    row.append(note);
  }
  return row;
}

function number(
  label: string,
  value: number,
  min: number,
  max: number,
  onChange: (value: number) => void,
): HTMLElement {
  const input = document.createElement("input");
  input.className = "settings-input settings-number";
  input.type = "number";
  input.min = String(min);
  input.max = String(max);
  input.value = String(value);
  input.addEventListener("input", () => {
    const parsed = Number(input.value);
    if (Number.isFinite(parsed)) onChange(Math.min(Math.max(parsed, min), max));
  });
  return row(label, input);
}

function text(
  label: string,
  value: string,
  placeholder: string,
  onChange: (value: string) => void,
  hint?: string,
): HTMLElement {
  const input = document.createElement("input");
  input.className = "settings-input";
  input.type = "text";
  input.value = value;
  input.placeholder = placeholder;
  input.addEventListener("input", () => onChange(input.value.trim()));
  return row(label, input, hint);
}

/** One of a few named options. */
function choice(
  label: string,
  value: string,
  options: [string, string][],
  onChange: (value: string) => void,
): HTMLElement {
  const select = document.createElement("select");
  select.className = "settings-input settings-select";
  for (const [id, title] of options) {
    const option = document.createElement("option");
    option.value = id;
    option.textContent = title;
    select.append(option);
  }
  select.value = options.some(([id]) => id === value) ? value : (options[0]?.[0] ?? "");
  select.addEventListener("change", () => onChange(select.value));
  return row(label, select);
}

function toggle(label: string, on: boolean, onChange: (on: boolean) => void): HTMLElement {
  const input = document.createElement("input");
  input.className = "settings-check";
  input.type = "checkbox";
  input.checked = on;
  input.addEventListener("change", () => onChange(input.checked));
  return row(label, input);
}

/** A language server's row: pre-warm, and how long it may idle. */
function serviceRow(
  label: string,
  prewarmed: boolean,
  onPrewarm: (on: boolean) => void,
  idle: string,
  onIdle: (value: string) => void,
): HTMLElement {
  const row = document.createElement("div");
  row.className = "settings-row";
  const caption = document.createElement("span");
  caption.className = "settings-label";
  caption.textContent = label;

  const warm = document.createElement("label");
  warm.className = "settings-inline";
  const check = document.createElement("input");
  check.type = "checkbox";
  check.className = "settings-check";
  check.checked = prewarmed;
  check.addEventListener("change", () => onPrewarm(check.checked));
  const warmLabel = document.createElement("span");
  warmLabel.textContent = "Start at launch";
  warm.append(check, warmLabel);

  const input = document.createElement("input");
  input.className = "settings-input settings-idle";
  input.type = "text";
  input.value = idle;
  input.placeholder = "the plugin's own";
  input.title = "How long it may sit idle: never, or 30m";
  input.addEventListener("input", () => onIdle(input.value.trim()));

  row.append(caption, warm, input);
  return row;
}

function button(label: string, onClick: () => void, kind = ""): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = `settings-button ${kind}`.trim();
  button.textContent = label;
  button.addEventListener("click", onClick);
  return button;
}

function spacer(): HTMLElement {
  const span = document.createElement("span");
  span.className = "settings-spacer";
  return span;
}

/** Put an idle override into a section, or take it out when it is emptied. */
function setIdle(section: Record<string, unknown>, id: string, value: string): void {
  if (value) section[id] = { idle_shutdown: value };
  else delete section[id];
}
