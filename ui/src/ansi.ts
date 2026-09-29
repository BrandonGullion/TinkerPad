// ANSI colours in program output (work item 04-05).
//
// Programs that colour their output — a test runner, a linter, anything using
// `colorama` or `Spectre.Console` — send SGR escape sequences. This turns them
// into styled runs of text; everything else an escape sequence might ask for
// (moving the cursor, clearing the screen) is dropped, because the output pane
// is a transcript, not a terminal.
//
// The parser is a state machine over chunks: a sequence split across two reads
// of the pipe is still one sequence, and colour set in one chunk still applies
// to the next.

/** How a run of text should look. Absent means "as the pane normally is". */
export interface Style {
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  /** Swap foreground and background, as `ESC[7m` asks. */
  inverse?: boolean;
}

/** A piece of text and how it should look. */
export interface Run {
  text: string;
  style: Style;
}

/** The sixteen colours, as CSS variables so each theme picks its own. */
const NAMES = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "bright-black",
  "bright-red",
  "bright-green",
  "bright-yellow",
  "bright-blue",
  "bright-magenta",
  "bright-cyan",
  "bright-white",
];

function named(index: number): string {
  return `var(--ansi-${NAMES[index] ?? "white"})`;
}

/** The 256-colour cube, as the usual xterm mapping. */
function cube(code: number): string {
  if (code < 16) return named(code);
  if (code < 232) {
    const n = code - 16;
    const steps = [0, 95, 135, 175, 215, 255];
    const r = steps[Math.floor(n / 36) % 6] ?? 0;
    const g = steps[Math.floor(n / 6) % 6] ?? 0;
    const b = steps[n % 6] ?? 0;
    return `rgb(${r} ${g} ${b})`;
  }
  const grey = 8 + (code - 232) * 10;
  return `rgb(${grey} ${grey} ${grey})`;
}

export class Ansi {
  private style: Style = {};
  /** A sequence that arrived split across chunks. */
  private partial = "";

  /** Whether anything has asked for a colour yet. */
  seen = false;

  /**
   * Turn a chunk of program output into runs. State carries across calls:
   * colour set in one chunk still applies to the next.
   */
  feed(chunk: string): Run[] {
    const text = this.partial + chunk;
    this.partial = "";
    const runs: Run[] = [];
    let plain = "";
    let at = 0;

    const flush = () => {
      if (plain) {
        runs.push({ text: plain, style: { ...this.style } });
        plain = "";
      }
    };

    while (at < text.length) {
      const esc = text.indexOf("", at);
      if (esc === -1) {
        plain += text.slice(at);
        break;
      }
      plain += text.slice(at, esc);

      const sequence = read(text, esc);
      if (sequence === null) {
        // The rest of it is in the next chunk.
        this.partial = text.slice(esc);
        break;
      }
      at = sequence.end;
      if (sequence.sgr !== null) {
        flush();
        this.apply(sequence.sgr);
        this.seen = true;
      }
      // Anything else — cursor moves, clears, OSC titles — is dropped.
    }
    flush();
    return runs;
  }

  /** Apply one SGR sequence's parameters. */
  private apply(params: string): void {
    const codes = params
      .split(";")
      .map((part) => (part === "" ? 0 : Number(part)))
      .filter((code) => Number.isFinite(code));

    for (let i = 0; i < codes.length; i++) {
      const code = codes[i] ?? 0;
      if (code === 0) this.style = {};
      else if (code === 1) this.style.bold = true;
      else if (code === 2) this.style.dim = true;
      else if (code === 3) this.style.italic = true;
      else if (code === 4) this.style.underline = true;
      else if (code === 7) this.style.inverse = true;
      else if (code === 22) this.style = { ...this.style, bold: false, dim: false };
      else if (code === 23) this.style.italic = false;
      else if (code === 24) this.style.underline = false;
      else if (code === 27) this.style.inverse = false;
      else if (code >= 30 && code <= 37) this.style.fg = named(code - 30);
      else if (code >= 90 && code <= 97) this.style.fg = named(code - 90 + 8);
      else if (code >= 40 && code <= 47) this.style.bg = named(code - 40);
      else if (code >= 100 && code <= 107) this.style.bg = named(code - 100 + 8);
      else if (code === 39) this.style.fg = undefined;
      else if (code === 49) this.style.bg = undefined;
      else if (code === 38 || code === 48) {
        // 5;n is the 256-colour cube; 2;r;g;b is a colour of its own.
        const kind = codes[i + 1];
        let colour: string | undefined;
        if (kind === 5) {
          colour = cube(codes[i + 2] ?? 0);
          i += 2;
        } else if (kind === 2) {
          const [r, g, b] = [codes[i + 2] ?? 0, codes[i + 3] ?? 0, codes[i + 4] ?? 0];
          colour = `rgb(${r} ${g} ${b})`;
          i += 4;
        }
        if (colour) {
          if (code === 38) this.style.fg = colour;
          else this.style.bg = colour;
        }
      }
    }
  }
}

/** Whether a style asks for anything at all. */
export function styled(style: Style): boolean {
  return Object.values(style).some((value) => value !== undefined && value !== false);
}

/** Put a style on an element. */
export function paint(span: HTMLElement, style: Style): void {
  const fg = style.inverse ? style.bg ?? "var(--bg)" : style.fg;
  const bg = style.inverse ? style.fg ?? "var(--fg)" : style.bg;
  if (fg) span.style.color = fg;
  if (bg) span.style.backgroundColor = bg;
  if (style.bold) span.style.fontWeight = "bold";
  if (style.dim) span.style.opacity = "0.7";
  if (style.italic) span.style.fontStyle = "italic";
  if (style.underline) span.style.textDecoration = "underline";
}

/**
 * Read the escape sequence starting at `esc`.
 *
 * Returns where it ends and, for an SGR sequence (`ESC[…m`), its parameters.
 * `null` means the sequence is incomplete — the rest is in the next chunk.
 */
function read(text: string, esc: number): { end: number; sgr: string | null } | null {
  const next = text[esc + 1];
  if (next === undefined) return null;

  // CSI: ESC [ params letter
  if (next === "[") {
    for (let i = esc + 2; i < text.length; i++) {
      const c = text[i] ?? "";
      if (c >= "@" && c <= "~") {
        return { end: i + 1, sgr: c === "m" ? text.slice(esc + 2, i) : null };
      }
    }
    return null;
  }
  // OSC: ESC ] … BEL or ESC \. The dump sentinel uses these, and it is read
  // before this ever sees the text (01-07); anything left is a window title.
  if (next === "]") {
    for (let i = esc + 2; i < text.length; i++) {
      if (text[i] === "") return { end: i + 1, sgr: null };
      if (text[i] === "" && text[i + 1] === "\\") return { end: i + 2, sgr: null };
    }
    return null;
  }
  // Anything else is a two-character sequence.
  return { end: esc + 2, sgr: null };
}
