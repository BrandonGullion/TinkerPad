// Open playgrounds as tabs over the one editor view (work item 01-12).
//
// Each tab keeps its own EditorState (document, undo history, selection),
// scroll position, output pane, and run state; switching tabs swaps them in.
// Edits autosave 500 ms after the last keystroke, and a run saves first. The
// open tabs are remembered in `<app-data>/session.json` and reopened at launch.
//
// The core is the source of truth for playground metadata: names, languages,
// and kinds are read from the library listing, never kept per tab.

import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

import { definitionKeymap } from "./definition";
import { annotationsExtension, annotationsOf, type EditorAnnotation, setAnnotations } from "./annotations";

import type { Editor } from "./editor";
import { invoke } from "./ipc";
import type { RunState, StatusBar } from "./layout";
import { forEachDiagnostic } from "@codemirror/lint";

import { findLocked, lockedLinesExtension } from "./locked";
import type { LspSession } from "./lsp";
import { OutputPane } from "./output";
import { pick, prompt } from "./picker";
import { LOCKED_HINT, MISMATCH_NOTE, type OpenedPlayground } from "./playground";
import type {
  AddPackage,
  Annotation,
  Listing,
  Peek,
  Playground,
  PlaygroundSummary,
  PluginsStatus,
  PluginSummary,
  Directive,
  Session,
} from "./protocol";
import type { RunController, RunHost } from "./run";
import { toast } from "./toast";

/** Autosave this long after the last edit. */
export const AUTOSAVE_MS = 500;
/** Session changes are written this long after the last one. */
const SESSION_SAVE_MS = 400;

/** How many recently opened playgrounds the session remembers (04-01). */
const RECENT_KEPT = 20;

const isMac = navigator.platform.toLowerCase().includes("mac");
const MOD = isMac ? "⌘" : "Ctrl+";

export interface TabElements {
  editor: Editor;
  status: StatusBar;
  tabStrip: HTMLElement;
  conflictBar: HTMLElement;
  editorHost: HTMLElement;
  emptyState: HTMLElement;
  /** The Output panel's content; the active tab's pane goes in it. */
  outputHost: HTMLElement;
}

export interface Tab {
  readonly id: string;
  readonly element: HTMLElement;
  readonly output: OutputPane;
  /** The editor state while inactive. The active tab's lives in the view. */
  state: EditorState;
  /** The text last saved to (or loaded from) disk. */
  savedText: string;
  dirty: boolean;
  /** The file changed on disk while this tab had unsaved edits. */
  conflict: boolean;
  scrollTop: number;
  /** A note for the status bar, such as why locked lines are off. */
  note: string | null;
  /** How the file was opened: its lock state and wrapper. */
  opened: Pick<OpenedPlayground, "prelude" | "postlude" | "lock">;
  saveTimer: number;
  saving: Promise<void> | null;
  /// Holds the highlighter and the language server's extensions, which arrive
  /// after the tab is already open and usable.
  language: Compartment;
  /// The tab's language server connection, once it is up (02-04).
  lsp: LspSession | null;
  /// Set while the connection is being made, so closing the tab can wait.
  connecting: Promise<void> | null;
  /// A file that isn't a playground, opened read-only (work item 02-09).
  /// Set on peek tabs and undefined on every other kind.
  peek?: PeekTab;
}

/** What a peek tab is showing. */
export interface PeekTab {
  /** The file on disk. */
  path: string;
  /** What the tab strip calls it: `folder/file.rs`. */
  title: string;
  /** Bytes on disk, and whether that was too many to load. */
  bytes: number;
  tooLarge: boolean;
}

/** How many read-only tabs are kept before the oldest is closed. */
export const MAX_PEEKS = 10;

export class TabManager implements RunHost {
  readonly tabs: Tab[] = [];
  /** Shown in the Output panel while no tab is open. */
  readonly emptyOutput: OutputPane;
  runs!: RunController;

  private active: Tab | null = null;
  private listing = new Map<string, PlaygroundSummary>();
  private plugins: PluginsStatus | null = null;
  private lastLanguage: string | null = null;
  /** Playgrounds opened lately, most recent first (work item 04-01). */
  private recentIds: string[] = [];
  private sessionTimer = 0;
  private restoring = false;
  private readonly newButton: HTMLButtonElement;

  constructor(private readonly ui: TabElements) {
    this.emptyOutput = new OutputPane(() => ui.editor.focus());
    this.newButton = document.createElement("button");
    this.newButton.className = "tab-new";
    this.newButton.textContent = "+";
    this.newButton.title = `New playground (${MOD}N)`;
    this.newButton.addEventListener("click", () => void this.newPlayground());
    ui.emptyState.textContent = `${MOD}N for a new playground, ${MOD}O to open one`;
    ui.editor.view.scrollDOM.addEventListener("scroll", () => this.saveSessionSoon(), { passive: true });
    this.showEmpty();
  }

  // ---- queries -------------------------------------------------------------

  get activeTab(): Tab | null {
    return this.active;
  }

  get activeId(): string | null {
    return this.active?.id ?? null;
  }

  get activeOutput(): OutputPane {
    return this.active?.output ?? this.emptyOutput;
  }

  find(id: string): Tab | undefined {
    return this.tabs.find((t) => t.id === id);
  }

  /** The plugin's display name for a language id, or the id itself. */
  languageName(id: string): string {
    return this.plugin(id)?.name ?? id;
  }

  /** The playground's metadata from the library listing. */
  summary(id: string): PlaygroundSummary | undefined {
    return this.listing.get(id);
  }

  /** A tab's current text, whether or not it's the active one. */
  textOf(tab: Tab): string {
    return (tab === this.active ? this.ui.editor.view.state : tab.state).doc.toString();
  }

  // ---- RunHost -------------------------------------------------------------

  activePlayground(): string | null {
    // A read-only file is not a playground, so there is nothing to run (02-09).
    return this.active?.peek ? null : this.activeId;
  }

  /** What to call the active playground: the run history's title (03-08). */
  activeName(): string | null {
    const id = this.activePlayground();
    return id === null ? null : (this.listing.get(id)?.name ?? null);
  }

  outputFor(playground: string): OutputPane | null {
    return this.find(playground)?.output ?? null;
  }

  beforeRun(playground: string, code: string): void {
    const tab = this.find(playground);
    if (!tab) return;
    // The run command writes `code` to disk itself.
    this.setAnnotations(tab, []);
    clearTimeout(tab.saveTimer);
    tab.saveTimer = 0;
    tab.savedText = code;
    this.updateDirty(tab);
  }

  runStateChanged(playground: string, state: RunState): void {
    const tab = this.find(playground);
    if (!tab) return;
    this.renderTab(tab);
    if (tab === this.active) this.ui.status.setRun(state);
  }

  flashNote(text: string): void {
    this.ui.status.flashNote(text);
  }

  /** A run's output pointed at a position: mark it if it's this tab's file. */
  annotation(playground: string, annotation: Annotation): void {
    const tab = this.find(playground);
    const summary = this.listing.get(playground);
    if (!tab || !summary || !sameFile(annotation.file, summary)) return;

    // A position inside the locked prelude can't be marked there, so mark the
    // first editable line and say where it really was.
    const lockedLines = tab.opened.lock === "locked" ? countLines(tab.opened.prelude) : 0;
    const inLocked = annotation.line <= lockedLines;
    const marked: EditorAnnotation = {
      line: inLocked ? lockedLines + 1 : annotation.line,
      col: inLocked ? null : annotation.col,
      message: annotation.message,
      severity: annotation.severity === "warning" ? "warning" : "error",
      note: inLocked ? `Reported on line ${annotation.line}, inside this kind's locked lines.` : undefined,
    };
    this.setAnnotations(tab, [...annotationsOf(this.stateOf(tab)), marked]);
  }

  /** The user clicked an annotation in the output pane. */
  revealAnnotation(playground: string, annotation: Annotation): void {
    const tab = this.find(playground);
    const summary = this.listing.get(playground);
    if (!tab || !summary) return;
    this.activate(tab);
    if (!sameFile(annotation.file, summary)) {
      this.ui.status.flashNote(`${annotation.file} isn't part of this playground, so there's nothing to open.`);
      return;
    }
    this.goTo(annotation.line, annotation.col);
  }

  /** Move to the next (`1`) or previous (`-1`) annotation in the active tab. */
  cycleAnnotation(delta: number): void {
    const tab = this.active;
    if (!tab) return;
    const list = this.marks();
    if (list.length === 0) {
      this.ui.status.flashNote("No errors or warnings to go to.");
      return;
    }
    const view = this.ui.editor.view;
    const current = view.state.doc.lineAt(view.state.selection.main.head).number;
    const next =
      delta > 0
        ? (list.find((a) => a.line > current) ?? list[0])
        : ([...list].reverse().find((a) => a.line < current) ?? list.at(-1));
    if (next) this.goTo(next.line, next.col, next.message);
  }

  /**
   * Everything worth walking with `F8` in the active tab: what the last run
   * reported (01-13) and what the language server is reporting now (02-04),
   * in document order. Both mark the same kind of thing, so one key walks
   * both rather than the user having to remember which is which.
   */
  private marks(): EditorAnnotation[] {
    const state = this.ui.editor.view.state;
    const marks: EditorAnnotation[] = [...annotationsOf(state)];
    forEachDiagnostic(state, (diagnostic, from) => {
      const line = state.doc.lineAt(from);
      marks.push({
        line: line.number,
        col: from - line.from + 1,
        message: diagnostic.message,
        severity: diagnostic.severity === "error" ? "error" : "warning",
      });
    });
    // A line can carry both a run error and a live diagnostic; one stop each
    // would mean pressing F8 twice to get anywhere.
    const seen = new Set<number>();
    return marks
      .sort((a, b) => a.line - b.line || (a.col ?? 0) - (b.col ?? 0))
      .filter((mark) => !seen.has(mark.line) && seen.add(mark.line));
  }

  /** Put the cursor on a line of the active tab and scroll it into view. */
  goTo(line: number, col: number | null, note?: string): void {
    const view = this.ui.editor.view;
    const target = view.state.doc.line(Math.min(Math.max(line, 1), view.state.doc.lines));
    const at = col ? Math.min(target.from + col - 1, target.to) : target.from;
    view.dispatch({
      selection: { anchor: at },
      effects: EditorView.scrollIntoView(at, { y: "center" }),
      scrollIntoView: true,
    });
    view.focus();
    if (note) this.ui.status.flashNote(note);
  }

  /** The editor state a tab's annotations live in. */
  private stateOf(tab: Tab): EditorState {
    return tab === this.active ? this.ui.editor.view.state : tab.state;
  }

  private setAnnotations(tab: Tab, list: EditorAnnotation[]): void {
    const effects = setAnnotations.of(list);
    if (tab === this.active) this.ui.editor.view.dispatch({ effects });
    else tab.state = tab.state.update({ effects }).state;
  }

  // ---- library -------------------------------------------------------------

  async refresh(): Promise<void> {
    const [listing, plugins] = await Promise.all([
      invoke<Listing>("list_playgrounds"),
      invoke<PluginsStatus>("list_plugins"),
    ]);
    this.plugins = plugins;
    this.setListing(listing);
  }

  async refreshListing(): Promise<void> {
    this.setListing(await invoke<Listing>("list_playgrounds"));
  }

  private setListing(listing: Listing): void {
    this.listing = new Map(listing.playgrounds.map((p) => [p.id, p]));
    for (const tab of this.tabs) this.renderTab(tab);
    this.renderStatus();
  }

  /** The library changed on disk: relabel tabs, and close ones whose playground is gone. */
  async libraryChanged(): Promise<void> {
    await this.refreshListing();
    for (const tab of [...this.tabs]) {
      if (tab.peek || this.listing.has(tab.id)) continue;
      toast("warning", "A playground that was open was removed from the library, so its tab was closed.");
      await this.close(tab, { save: false });
    }
  }

  private plugin(id: string): PluginSummary | undefined {
    return this.plugins?.plugins.find((p) => p.id === id);
  }

  // ---- opening, switching, closing ----------------------------------------

  /** Playgrounds opened lately, most recent first (work item 04-01). */
  recent(): string[] {
    return this.recentIds;
  }

  /** A playground is gone: close its tab, if it has one. */
  closed(id: string): void {
    this.recentIds = this.recentIds.filter((known) => known !== id);
    const tab = this.find(id);
    if (tab) void this.close(tab, { save: false });
  }

  /** Note that a playground was opened, for the Recent section. */
  private remember(id: string): void {
    this.recentIds = [id, ...this.recentIds.filter((known) => known !== id)].slice(0, RECENT_KEPT);
    this.saveSessionSoon();
  }

  /** Open a playground in a tab (or switch to its tab) and make it active. */
  async open(id: string, activate = true): Promise<OpenedPlayground> {
    this.remember(id);
    const existing = this.find(id);
    if (existing) {
      if (activate) this.activate(existing);
      return this.openedFromTab(existing);
    }
    const opened = await invoke<OpenedPlayground>("open_playground", { id });
    if (!this.plugins) this.plugins = await invoke<PluginsStatus>("list_plugins");
    if (!this.listing.has(id)) await this.refreshListing();
    const again = this.find(id);
    if (again) {
      // Opened meanwhile by another call.
      if (activate) this.activate(again);
      return opened;
    }

    const element = document.createElement("div");
    const tab: Tab = {
      id,
      element,
      output: new OutputPane(() => this.ui.editor.focus()),
      state: this.ui.editor.createState(""),
      savedText: opened.text,
      dirty: false,
      conflict: false,
      scrollTop: 0,
      note: null,
      opened: { prelude: null, postlude: null, lock: "none" },
      saveTimer: 0,
      saving: null,
      language: new Compartment(),
      lsp: null,
      connecting: null,
    };
    this.load(tab, opened);
    void this.connectLanguage(tab);
    this.buildTabElement(tab);
    this.tabs.push(tab);
    this.ui.tabStrip.replaceChildren(...this.tabs.map((t) => t.element), this.newButton);
    this.renderTab(tab);
    if (activate) this.activate(tab);
    this.saveSessionSoon();
    return opened;
  }

  /**
   * Open a file that isn't a playground, read-only: a dependency's source, a
   * generated file, later a stack frame (work item 02-09, DEBUGGING.md D15).
   *
   * The tab has no playground behind it, so nothing saves, runs, or attaches a
   * language server to it; closing it costs nothing. Ten of them is plenty, and
   * the oldest goes when an eleventh arrives.
   */
  async openReadOnly(path: string, line?: number, col?: number): Promise<Tab | null> {
    const existing = this.tabs.find((tab) => tab.peek?.path === path);
    if (existing) {
      this.activate(existing);
      if (line) this.goTo(line, col ?? null);
      return existing;
    }

    let peek: Peek;
    try {
      peek = await invoke<Peek>("open_readonly", { path });
    } catch {
      return null;
    }

    // Room for one more.
    const peeks = this.tabs.filter((tab) => tab.peek);
    for (const old of peeks.slice(0, Math.max(0, peeks.length - (MAX_PEEKS - 1)))) {
      await this.close(old, { save: false });
    }

    const element = document.createElement("div");
    const tab: Tab = {
      id: `peek:${path}`,
      element,
      output: new OutputPane(() => this.ui.editor.focus()),
      state: this.ui.editor.createState(""),
      savedText: peek.text,
      dirty: false,
      conflict: false,
      scrollTop: 0,
      note: null,
      opened: { prelude: null, postlude: null, lock: "none" },
      saveTimer: 0,
      saving: null,
      language: new Compartment(),
      lsp: null,
      connecting: null,
      peek: {
        path: peek.path,
        title: peek.title,
        bytes: peek.bytes,
        tooLarge: peek.tooLarge,
      },
    };

    const text = peek.tooLarge
      ? `${peek.path}\n\nThis file is ${Math.round(peek.bytes / (1024 * 1024))} MB, which is more than TinkerPad will open for reading. Open it in your editor instead.\n`
      : peek.text;
    tab.state = this.ui.editor.createState(text, [
      // Read-only, and visibly so: this is someone else's file.
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      tab.language.of([]),
    ]);

    this.buildTabElement(tab);
    this.tabs.push(tab);
    this.ui.tabStrip.replaceChildren(...this.tabs.map((t) => t.element), this.newButton);
    this.renderTab(tab);
    this.activate(tab);
    if (line && !peek.tooLarge) this.goTo(line, col ?? null);

    // The highlighter comes from the extension, since there is no plugin.
    void import("./highlight").then(async ({ highlighterFor }) => {
      const syntax = await highlighterFor(peek.extension);
      if (syntax && this.tabs.includes(tab)) this.reconfigure(tab, syntax);
    });
    return tab;
  }

  /** What `open` reports for an already open tab. */
  private openedFromTab(tab: Tab): OpenedPlayground {
    const summary = this.listing.get(tab.id);
    return {
      playground: summary as Playground,
      text: this.textOf(tab),
      ...tab.opened,
    };
  }

  /** Give `tab` a fresh editor state for `opened`, locking lines as needed. */
  private load(tab: Tab, opened: OpenedPlayground): void {
    const ranges =
      opened.lock === "locked" ? findLocked(opened.text, opened.prelude ?? "", opened.postlude ?? "") : null;
    tab.state = this.ui.editor.createState(opened.text, [
      lockedLinesExtension(ranges, () => this.ui.status.flashNote(LOCKED_HINT)),
      annotationsExtension(),
      tab.language.of([]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) this.edited(tab);
      }),
    ]);
    tab.savedText = opened.text;
    tab.dirty = false;
    tab.opened = { prelude: opened.prelude, postlude: opened.postlude, lock: opened.lock };
    tab.note = opened.lock === "mismatch" || (opened.lock === "locked" && !ranges) ? MISMATCH_NOTE : null;
  }

  /**
   * Load the tab's highlighter and connect it to its language server, then
   * reconfigure the tab with what came back.
   *
   * This runs after the tab is open on purpose: a cold rust-analyzer takes
   * seconds to answer its handshake, and a tab must never wait for one. The
   * editor works throughout; the language features appear when they appear.
   */
  private async connectLanguage(tab: Tab): Promise<void> {
    const summary = this.listing.get(tab.id);
    const plugin = summary && this.plugin(summary.language);
    if (!summary) return;
    // The LSP client is a chunk of its own: a window that only ever shows
    // plain-text playgrounds never downloads it.
    const work = import("./lsp")
      .then(({ languageExtensions }) =>
        languageExtensions(
          tab.id,
          plugin?.highlighter ?? null,
          plugin?.lsp ?? false,
          // F12 and friends only mean anything with a server behind them.
          definitionKeymap(this),
        ),
      )
      .then(({ extensions, session }) => {
        // The tab may have been closed while the server was starting.
        if (!this.tabs.includes(tab)) {
          void session?.close();
          return;
        }
        tab.lsp = session;
        this.reconfigure(tab, extensions);
      })
      .catch((error: unknown) => {
        console.warn("language support failed to load", error);
      })
      .finally(() => {
        tab.connecting = null;
      });
    tab.connecting = work;
    await work;
  }

  /** Put an extension into the tab's language compartment, wherever it lives. */
  private reconfigure(tab: Tab, extensions: Extension): void {
    const effects = tab.language.reconfigure(extensions);
    if (tab === this.active) this.ui.editor.view.dispatch({ effects });
    else tab.state = tab.state.update({ effects }).state;
  }

  activate(tab: Tab): void {
    if (!this.tabs.includes(tab)) return;
    if (this.active !== tab) {
      this.stash();
      this.active = tab;
      this.ui.editorHost.classList.remove("empty");
      this.ui.emptyState.hidden = true;
      this.ui.editor.setState(tab.state);
      this.restoreScroll(tab.scrollTop);
      this.ui.outputHost.replaceChildren(tab.output.element);
      tab.output.attached();
      const language = this.listing.get(tab.id)?.language;
      if (language) this.lastLanguage = language;
    }
    for (const t of this.tabs) this.renderTab(t);
    this.renderStatus();
    this.renderConflict();
    this.saveSessionSoon();
  }

  /** Keep the active tab's editor state and scroll before swapping it out. */
  private stash(): void {
    if (!this.active) return;
    this.active.state = this.ui.editor.view.state;
    this.active.scrollTop = this.ui.editor.view.scrollDOM.scrollTop;
  }

  private restoreScroll(top: number): void {
    const scroller = this.ui.editor.view.scrollDOM;
    scroller.scrollTop = top;
    // The view measures its content asynchronously; set it again once it has.
    requestAnimationFrame(() => {
      scroller.scrollTop = top;
      requestAnimationFrame(() => (scroller.scrollTop = top));
    });
  }

  /** Move to the next (`1`) or previous (`-1`) tab, wrapping around. */
  cycle(delta: number): void {
    if (this.tabs.length === 0) return;
    const index = this.active ? this.tabs.indexOf(this.active) : -1;
    const next = this.tabs[(index + delta + this.tabs.length) % this.tabs.length];
    if (next) this.activate(next);
  }

  /** Close a tab, saving it first unless told not to. */
  async close(tab: Tab, { save = true } = {}): Promise<void> {
    const index = this.tabs.indexOf(tab);
    if (index < 0) return;
    if (save) await this.flush(tab);
    clearTimeout(tab.saveTimer);
    if (this.runs.isRunning(tab.id)) void this.runs.cancel(tab.id);
    this.runs.forget(tab.id);
    this.tabs.splice(index, 1);
    tab.element.remove();
    tab.output.clear();
    // The plugin may want to know (03-07); it is nothing the user waits for.
    void invoke("close_playground", { id: tab.id }).catch(() => {});
    // Closing the document leaves the server running for the next tab (02-02).
    void this.disconnectLanguage(tab);
    if (this.active === tab) {
      this.active = null;
      const next = this.tabs[index] ?? this.tabs[index - 1];
      if (next) this.activate(next);
      else this.showEmpty();
    }
    this.saveSessionSoon();
  }

  /** Close a tab's language server connection, once it has finished opening. */
  private async disconnectLanguage(tab: Tab): Promise<void> {
    if (tab.connecting) await tab.connecting.catch(() => {});
    const session = tab.lsp;
    tab.lsp = null;
    await session?.close();
  }

  async closeActive(): Promise<void> {
    if (this.active) await this.close(this.active);
  }

  /** No tabs: hide the editor and show how to get started. */
  showEmpty(): void {
    this.stash();
    this.active = null;
    this.ui.editor.setState(this.ui.editor.createState(""));
    this.ui.editorHost.classList.add("empty");
    this.ui.emptyState.hidden = false;
    this.ui.outputHost.replaceChildren(this.emptyOutput.element);
    this.ui.status.setLanguage("No playground");
    this.ui.status.setNote(null);
    this.ui.status.setRun({ kind: "idle" });
    // Nothing is open, so a stopped server is nobody's business (02-08).
    this.ui.status.servers.setLanguages([]);
    this.renderConflict();
  }

  // ---- creating ------------------------------------------------------------

  /** The language Mod+N uses: the active tab's, the last used, or the first loaded. */
  defaultLanguage(): string | null {
    const loaded = (id: string | null | undefined) => (id && this.plugin(id) ? id : null);
    return (
      loaded(this.active ? this.listing.get(this.active.id)?.language : null) ??
      loaded(this.lastLanguage) ??
      this.plugins?.plugins[0]?.id ??
      null
    );
  }

  /** `Untitled N` with the smallest N not already used in the library. */
  untitledName(): string {
    const taken = new Set([...this.listing.values()].map((p) => p.name.toLowerCase()));
    for (let n = 1; ; n++) {
      const name = `Untitled ${n}`;
      if (!taken.has(name.toLowerCase())) return name;
    }
  }

  /** Create a playground (last used language, default kind unless given) and open it. */
  async newPlayground(language?: string, kind?: string): Promise<Playground | null> {
    await this.refresh();
    const lang = language ?? this.defaultLanguage();
    if (!lang) {
      toast("warning", "No language plugins are loaded, so there's nothing to create a playground with.");
      return null;
    }
    const playground = await invoke<Playground>("create_playground", {
      name: this.untitledName(),
      language: lang,
      kind: kind ?? null,
    });
    this.lastLanguage = lang;
    await this.refreshListing();
    await this.open(playground.id);
    this.ui.editor.focus();
    return playground;
  }

  /** Mod+Shift+N: pick a language, then a kind, then create. */
  async pickAndCreate(): Promise<Playground | null> {
    await this.refresh();
    const plugins = [...(this.plugins?.plugins ?? [])];
    const preferred = this.defaultLanguage();
    plugins.sort((a, b) => Number(b.id === preferred) - Number(a.id === preferred));
    const language = await pick(
      "New playground",
      plugins.map((p) => ({
        id: p.id,
        label: p.name,
        detail: p.kinds.length > 1 ? `${p.kinds.length} kinds` : undefined,
      })),
      "Language",
    );
    const plugin = language && this.plugin(language.id);
    if (!plugin) return null;
    let kind = plugin.kinds.find((k) => k.default) ?? plugin.kinds[0];
    if (plugin.kinds.length > 1) {
      const kinds = [...plugin.kinds].sort((a, b) => Number(b.default) - Number(a.default));
      const chosen = await pick(
        `New ${plugin.name} playground`,
        kinds.map((k) => ({ id: k.id, label: k.name, detail: k.default ? "default" : undefined })),
        "Kind",
      );
      if (!chosen) return null;
      kind = plugin.kinds.find((k) => k.id === chosen.id);
    }
    return this.newPlayground(plugin.id, kind?.id);
  }

  /** Mod+O: pick any playground in the library and open it. */
  async quickOpen(): Promise<void> {
    await this.refresh();
    const loaded = new Set(this.plugins?.plugins.map((p) => p.id));
    const choice = await pick(
      "Open playground",
      [...this.listing.values()].map((p) => ({
        id: p.id,
        label: p.name,
        detail: `${this.plugin(p.language)?.name ?? p.language}${loaded.has(p.language) ? "" : " (not loaded)"}${p.preview ? ` · ${p.preview}` : ""}`,
      })),
      "Name or language",
    );
    if (!choice) return;
    await this.open(choice.id);
    this.ui.editor.focus();
  }

  // ---- packages ------------------------------------------------------------

  /**
   * Add a package to the active playground, the way its plugin's `[deps]`
   * says (work item 01-14). Either a command runs, with its output in the
   * tab's pane, or a directive goes at the top of the file.
   */
  async addPackage(): Promise<void> {
    const tab = this.active;
    const summary = tab && this.listing.get(tab.id);
    if (!tab || !summary) {
      this.ui.status.flashNote("Open a playground to add a package to it.");
      return;
    }
    const answers = await prompt(`Add a package to ${summary.name}`, [
      { label: "Package", placeholder: "name" },
      { label: "Version", placeholder: "latest", optional: true },
    ]);
    const [name, version] = answers ?? [];
    if (!name) return;

    // A push, not a variable, so the result of the callback below is still
    // typed when it's read after the await.
    const directives: Directive[] = [];
    const handled: string[][] = [];
    tab.output.clear();
    await this.runs
      .adopt(tab.id, async () => {
        const result = await invoke<AddPackage>("add_package", {
          playground: tab.id,
          name,
          version: version || null,
        });
        if (result.kind === "directive") {
          directives.push(result);
          return null;
        }
        if (result.kind === "handled") {
          handled.push(result.files);
          return null;
        }
        tab.output.append("info", `$ ${result.command}\n`);
        return result.run;
      })
      .catch(() => {});
    const directive = directives[0];
    if (directive) this.insertDirective(tab, name, directive);
    const files = handled[0];
    if (files) {
      const what = files.length ? `Added ${name}: ${files.join(", ")}.` : `Added ${name}.`;
      tab.output.append("info", `${what}
`);
      this.ui.status.flashNote(what);
      // The hook may have rewritten the file itself; don't take the user's
      // unsaved edits with it. A dirty tab gets the conflict bar instead.
      if (!tab.dirty) void this.reload(tab);
    }
  }

  /**
   * Put the plugin's directive line at the top of the file, after any
   * directives already there. A line for the same package is replaced, so
   * adding twice doesn't leave two.
   */
  private insertDirective(tab: Tab, name: string, directive: Directive): void {
    // The directive has to be the first thing in the file, and a kind with a
    // prelude owns that spot: its lines are locked, and the file has to start
    // with them for the lock to be recognised at all.
    if (tab.opened.lock === "locked" && tab.opened.prelude) {
      toast(
        "warning",
        `A package line has to go at the top of the file, and this kind's first lines are locked. ` +
          `Add ${name} from a playground of a kind without locked lines.`,
      );
      return;
    }
    if (tab !== this.active) return;

    const view = this.ui.editor.view;
    const doc = view.state.doc;
    const { prefix, line } = directive;
    const mine = `${prefix}${name}`;
    let insertBefore = 1;
    for (let n = 1; n <= doc.lines; n++) {
      const existing = doc.line(n);
      if (!existing.text.startsWith(prefix)) break;
      const rest = existing.text.slice(mine.length);
      if (existing.text.startsWith(mine) && (rest === "" || rest.startsWith("@"))) {
        if (existing.text === line) {
          this.ui.status.flashNote(`${name} is already in this playground.`);
          return;
        }
        view.dispatch({ changes: { from: existing.from, to: existing.to, insert: line } });
        this.afterDirective(tab, `Updated ${name}.`);
        return;
      }
      insertBefore = n + 1;
    }
    const at = insertBefore <= doc.lines ? doc.line(insertBefore).from : doc.length;
    const insert = insertBefore <= doc.lines ? `${line}\n` : `\n${line}\n`;
    view.dispatch({ changes: { from: at, insert } });
    this.afterDirective(tab, `Added ${line.trim()}.`);
  }

  /** Save the directive straight away, so the next run compiles with it. */
  private afterDirective(tab: Tab, note: string): void {
    this.ui.status.flashNote(note);
    tab.output.append("info", `${note}\n`);
    void this.save(tab);
  }

  // ---- saving --------------------------------------------------------------

  private edited(tab: Tab): void {
    this.updateDirty(tab);
    clearTimeout(tab.saveTimer);
    tab.saveTimer = tab.dirty ? window.setTimeout(() => void this.save(tab), AUTOSAVE_MS) : 0;
  }

  private updateDirty(tab: Tab): void {
    const dirty = this.textOf(tab) !== tab.savedText;
    if (dirty === tab.dirty) return;
    tab.dirty = dirty;
    this.renderTab(tab);
  }

  /** Save a tab now if it has unsaved edits. */
  async save(tab: Tab): Promise<void> {
    clearTimeout(tab.saveTimer);
    tab.saveTimer = 0;
    if (!this.tabs.includes(tab) || tab.conflict || tab.peek) return;
    if (tab.saving) await tab.saving;
    const text = this.textOf(tab);
    if (text === tab.savedText) {
      this.updateDirty(tab);
      return;
    }
    tab.saving = invoke<boolean>("write_source", { id: tab.id, text })
      .then(() => {
        tab.savedText = text;
        this.updateDirty(tab);
      })
      .catch(() => {
        // The error was shown as a toast; the tab stays dirty.
      })
      .finally(() => {
        tab.saving = null;
      });
    await tab.saving;
  }

  /** Wait for any pending or in-flight save of `tab`. */
  async flush(tab: Tab): Promise<void> {
    if (tab.saving) await tab.saving;
    if (tab.saveTimer || tab.dirty) await this.save(tab);
  }

  async flushAll(): Promise<void> {
    await Promise.all(this.tabs.map((t) => this.flush(t)));
  }

  // ---- changes on disk -----------------------------------------------------

  /** A playground's main file changed outside the app. */
  async sourceChanged(id: string): Promise<void> {
    const tab = this.find(id);
    if (!tab) return;
    if (tab.dirty || tab.saveTimer) {
      clearTimeout(tab.saveTimer);
      tab.saveTimer = 0;
      tab.conflict = true;
      this.renderConflict();
    } else {
      await this.reload(tab);
    }
  }

  /** Replace a tab's text with what's on disk. */
  async reload(tab: Tab): Promise<void> {
    const opened = await invoke<OpenedPlayground>("open_playground", { id: tab.id });
    const scroll = tab === this.active ? this.ui.editor.view.scrollDOM.scrollTop : tab.scrollTop;
    tab.conflict = false;
    this.load(tab, opened);
    if (tab === this.active) {
      this.ui.editor.setState(tab.state);
      this.restoreScroll(scroll);
    }
    this.renderTab(tab);
    this.renderStatus();
    this.renderConflict();
  }

  /** Keep the tab's text and write it over the change on disk. */
  async keepMine(tab: Tab): Promise<void> {
    tab.conflict = false;
    this.renderConflict();
    const text = this.textOf(tab);
    await invoke<boolean>("write_source", { id: tab.id, text });
    tab.savedText = text;
    this.updateDirty(tab);
  }

  // ---- renaming ------------------------------------------------------------

  /** Rename a tab's playground inline (double-click or F2). */
  startRename(tab: Tab | null = this.active): void {
    if (!tab || tab.element.querySelector(".tab-rename")) return;
    const label = tab.element.querySelector<HTMLElement>(".tab-name")!;
    const current = this.listing.get(tab.id)?.name ?? label.textContent ?? "";
    const input = document.createElement("input");
    input.className = "tab-rename";
    input.value = current;
    input.setAttribute("aria-label", "Playground name");
    label.hidden = true;
    label.after(input);
    input.focus();
    input.select();

    let done = false;
    const finish = async (commit: boolean) => {
      if (done) return;
      done = true;
      const name = input.value.trim();
      input.remove();
      label.hidden = false;
      if (commit && name && name !== current) {
        try {
          await invoke<Playground>("rename_playground", { id: tab.id, name });
          await this.refreshListing();
        } catch {
          // Shown as a toast.
        }
      }
      if (tab === this.active) this.ui.editor.focus();
    };
    input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") {
        e.preventDefault();
        void finish(true);
      } else if (e.key === "Escape") {
        e.preventDefault();
        void finish(false);
      }
    });
    input.addEventListener("blur", () => void finish(true));
    input.addEventListener("mousedown", (e) => e.stopPropagation());
    input.addEventListener("dblclick", (e) => e.stopPropagation());
  }

  // ---- session -------------------------------------------------------------

  snapshot(): Session {
    return {
      tabs: this.tabs
        .filter((t) => !t.peek)
        .map((t) => ({
          playground: t.id,
          scrollTop: t === this.active ? this.ui.editor.view.scrollDOM.scrollTop : t.scrollTop,
        })),
      active: this.active?.peek ? null : this.activeId,
      lastLanguage: this.lastLanguage,
      recent: this.recentIds.slice(0, RECENT_KEPT),
    };
  }

  saveSessionSoon(): void {
    if (this.restoring) return;
    clearTimeout(this.sessionTimer);
    this.sessionTimer = window.setTimeout(() => void this.saveSession(), SESSION_SAVE_MS);
  }

  async saveSession(): Promise<void> {
    clearTimeout(this.sessionTimer);
    await invoke("save_session", { session: this.snapshot() }).catch(() => {});
  }

  /** Reopen the tabs of the last session, skipping playgrounds that are gone. */
  async restoreSession(session?: Session): Promise<{ opened: number; skipped: number }> {
    this.restoring = true;
    let opened = 0;
    let skipped = 0;
    try {
      const saved = session ?? (await invoke<Session>("load_session"));
      await this.refresh();
      this.lastLanguage = saved.lastLanguage ?? this.lastLanguage;
      this.recentIds = saved.recent ?? [];
      for (const entry of saved.tabs) {
        const summary = this.listing.get(entry.playground);
        if (!summary || !this.plugin(summary.language)) {
          skipped++;
          continue;
        }
        try {
          await this.open(entry.playground, false);
          const tab = this.find(entry.playground);
          if (tab) tab.scrollTop = entry.scrollTop;
          opened++;
        } catch {
          skipped++;
        }
      }
      const target = (saved.active && this.find(saved.active)) || this.tabs[0];
      if (target) this.activate(target);
      else if (!this.active) this.showEmpty();
    } finally {
      this.restoring = false;
    }
    if (skipped > 0) {
      toast(
        "warning",
        skipped === 1
          ? "One playground from your last session is no longer in the library, so it wasn't reopened."
          : `${skipped} playgrounds from your last session are no longer in the library, so they weren't reopened.`,
      );
    }
    this.saveSessionSoon();
    return { opened, skipped };
  }

  // ---- rendering -----------------------------------------------------------

  private buildTabElement(tab: Tab): void {
    const el = tab.element;
    el.className = "tab";
    el.setAttribute("role", "tab");
    el.dataset.id = tab.id;
    const lang = document.createElement("span");
    lang.className = "tab-lang";
    const name = document.createElement("span");
    name.className = "tab-name";
    const dirty = document.createElement("span");
    dirty.className = "tab-dirty";
    dirty.textContent = "●";
    dirty.title = "Unsaved changes";
    const running = document.createElement("span");
    running.className = "tab-running";
    running.title = "Running";
    const close = document.createElement("button");
    close.className = "tab-close";
    close.textContent = "×";
    close.title = `Close (${MOD}W)`;
    close.setAttribute("aria-label", "Close tab");
    el.append(lang, name, dirty, running, close);

    close.addEventListener("click", (e) => {
      e.stopPropagation();
      void this.close(tab);
    });
    el.addEventListener("mousedown", (e) => {
      // Middle click closes; stop the browser's autoscroll from starting.
      if (e.button === 1) e.preventDefault();
    });
    el.addEventListener("auxclick", (e) => {
      if (e.button !== 1) return;
      e.preventDefault();
      void this.close(tab);
    });
    el.addEventListener("click", () => {
      this.activate(tab);
      this.ui.editor.focus();
    });
    name.addEventListener("dblclick", (e) => {
      e.stopPropagation();
      this.startRename(tab);
    });
  }

  private renderTab(tab: Tab): void {
    if (tab.peek) {
      const el = tab.element;
      el.querySelector<HTMLElement>(".tab-name")!.textContent = tab.peek.title;
      el.querySelector<HTMLElement>(".tab-lang")!.textContent = "🔒";
      el.querySelector<HTMLElement>(".tab-dirty")!.hidden = true;
      el.querySelector<HTMLElement>(".tab-running")!.hidden = true;
      el.classList.toggle("active", tab === this.active);
      el.classList.add("peek");
      el.setAttribute("aria-selected", String(tab === this.active));
      el.title = `${tab.peek.path} · read-only`;
      return;
    }
    const summary = this.listing.get(tab.id);
    const plugin = summary ? this.plugin(summary.language) : undefined;
    const el = tab.element;
    const nameEl = el.querySelector<HTMLElement>(".tab-name");
    if (!nameEl) return;
    const name = summary?.name ?? "…";
    nameEl.textContent = name;
    el.querySelector<HTMLElement>(".tab-lang")!.textContent = languageCode(plugin, summary?.language);
    el.querySelector<HTMLElement>(".tab-dirty")!.hidden = !tab.dirty;
    const running = this.runs?.isRunning(tab.id) ?? false;
    el.querySelector<HTMLElement>(".tab-running")!.hidden = !running;
    el.classList.toggle("active", tab === this.active);
    el.classList.toggle("dirty", tab.dirty);
    el.classList.toggle("running", running);
    el.setAttribute("aria-selected", String(tab === this.active));
    el.title = `${name} · ${plugin?.name ?? summary?.language ?? ""}`;
  }

  private renderStatus(): void {
    // The status bar shows a stopped server only while a tab still wants it
    // (02-08), so it needs to know which languages are open.
    this.ui.status.servers.setLanguages(
      this.tabs.map((t) => this.listing.get(t.id)?.language).filter((l): l is string => !!l),
    );
    const tab = this.active;
    if (!tab) return;
    if (tab.peek) {
      this.ui.status.setLanguage("Read-only");
      this.ui.status.setNote(tab.peek.path);
      this.ui.status.setRun({ kind: "idle" });
      return;
    }
    const summary = this.listing.get(tab.id);
    this.ui.status.setLanguage(summary ? `${summary.language} · ${summary.kind}` : "…");
    this.ui.status.setNote(tab.note);
    this.ui.status.setRun(this.runs?.stateOf(tab.id) ?? { kind: "idle" });
  }

  private renderConflict(): void {
    const bar = this.ui.conflictBar;
    const tab = this.active;
    if (!tab?.conflict) {
      bar.hidden = true;
      bar.replaceChildren();
      return;
    }
    const message = document.createElement("span");
    message.textContent = `“${this.listing.get(tab.id)?.name ?? "This playground"}” changed on disk while you had unsaved edits.`;
    const reload = document.createElement("button");
    reload.className = "conflict-reload";
    reload.textContent = "Reload";
    reload.title = "Discard your edits and load the file from disk";
    reload.addEventListener("click", () => void this.reload(tab));
    const keep = document.createElement("button");
    keep.className = "conflict-keep";
    keep.textContent = "Keep mine";
    keep.title = "Keep your edits and save them over the file";
    keep.addEventListener("click", () => void this.keepMine(tab));
    bar.replaceChildren(message, reload, keep);
    bar.hidden = false;
  }
}

/** Lines in a kind's prelude, which are locked at the top of the file. */
function countLines(prelude: string | null): number {
  return prelude ? prelude.split("\n").length - 1 : 0;
}

/**
 * Whether an annotation's file is the playground's main file. A pattern with
 * no `file` group (or an empty one) means "here". Tools write paths as
 * absolute, relative to the playground, or as `file://` URLs.
 */
export function sameFile(file: string | null, playground: { mainFile: string; dir: string }): boolean {
  if (!file?.trim()) return true;
  let text = file.trim();
  if (text.startsWith("file:")) {
    try {
      text = decodeURIComponent(text.replace(/^file:\/\/\/?/, ""));
    } catch {
      text = text.replace(/^file:\/\/\/?/, "");
    }
  }
  const slashes = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
  const path = slashes(text);
  const dir = slashes(playground.dir);
  const absolute = /^([a-zA-Z]:\/|\/)/.test(path) ? path : `${dir}/${path.replace(/^\.\//, "")}`;
  const same = (a: string, b: string) => (isWindows ? a.toLowerCase() === b.toLowerCase() : a === b);
  return same(absolute, slashes(playground.mainFile));
}

const isWindows = navigator.platform.toLowerCase().includes("win");

/** A short language badge: `C#`, or the extension (`TS`, `RS`). */
function languageCode(plugin: PluginSummary | undefined, language: string | undefined): string {
  if (plugin) return plugin.name.length <= 3 ? plugin.name : plugin.extension.toUpperCase().slice(0, 3);
  return (language ?? "?").slice(0, 2).toUpperCase();
}
