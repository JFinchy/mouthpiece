// Roadmap node graph. Everything on the page is drawn from the three arrays
// below: edit NODES/EDGES to update it, no other code needs to change.
//
//   col    which column (STAGES index) the node sits in, left to right
//   lane   which row band (LANES index) it belongs to
//   status shipped | testing | progress | planned | research
//   order  optional priority within the "Next up" list on the page

const STAGES = [
  { head: "Shipped", sub: "Foundations" },
  { head: "Shipped", sub: "Agents era" },
  { head: "In testing", sub: "Built, verifying" },
  { head: "Now & next", sub: "Agent fleet" },
  { head: "Then", sub: "Polish & teams" },
  { head: "Later", sub: "Design & research" },
];

const LANES = ["Agents", "Context", "Dictation", "Modes & automation", "Voice & meetings", "Trust & local AI"];

const STATUS = {
  shipped: { label: "Shipped" },
  testing: { label: "In testing" },
  progress: { label: "In progress" },
  planned: { label: "Planned" },
  research: { label: "Research" },
};

const NODES = [
  // ---- Agents
  {
    id: "agent-commands", label: "Agent voice commands", title: "Agent voice commands",
    lane: 0, col: 1, status: "shipped",
    what: "“Send it” presses Enter, “attach the error” adds your latest stack trace as a code block, “scratch that” safely undoes the last paste, and “append clipboard” adds what you copied.",
    why: "The things you'd otherwise reach for the keyboard for while prompting an agent.",
    note: "Shipped in 0.3.", source: "FEATURES.md",
  },
  {
    id: "reply", label: "Reply to an agent", title: "Reply to a waiting agent",
    lane: 0, col: 1, status: "shipped",
    what: "Claude Code hooks tell Mouthpiece when an agent is waiting. The widget shows a pill; the reply shortcut (⌃⌥R) records your answer, pastes it into that agent's window and returns you to where you were.",
    why: "Answering an agent shouldn't mean finding its terminal tab.",
    note: "Shipped in 0.3. Works best in Terminal, VS Code and Cursor, whose window titles name the folder.", source: "FEATURES.md",
  },
  {
    id: "fleet", label: "Many agents at once", title: "Several agents at once",
    lane: 0, col: 3, status: "progress", order: 1,
    what: "Address an agent by its folder name (“to esc-pause: rerun the tests, send it”), a menu of waiting agents instead of a single pill, replies into Orca terminals through <code>orca terminal send</code>, and the right iTerm or Terminal tab picked by its tty.",
    why: "Running several coding agents in parallel is becoming normal; the voice layer has to know which one you mean.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "ROADMAP.md",
  },
  {
    id: "mcp", label: "CLI + MCP server", title: "mouthpiece CLI and MCP server",
    lane: 0, col: 3, status: "progress", order: 2,
    what: "<code>mouthpiece listen</code> prints a transcript to stdout. An MCP <code>ask_user</code> tool lets an agent ask you a question by voice, and <code>add_vocabulary</code> lets it teach Mouthpiece project terms.",
    why: "Lets agents and scripts use Mouthpiece directly, instead of only receiving pasted text.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "ROADMAP.md",
  },
  {
    id: "more-hooks", label: "Hooks for more agents", title: "Hooks for more agents",
    lane: 0, col: 3, status: "progress", order: 4,
    what: "The waiting pill and reply shortcut for Codex, Gemini CLI, OpenCode and Aider, not just Claude Code.",
    why: "Reply-to-agent shouldn't depend on which agent you use.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "ROADMAP.md",
  },
  {
    id: "more-errors", label: "More error sources", title: "“Attach the error” from more terminals",
    lane: 0, col: 4, status: "planned", order: 7,
    what: "Read the latest error from Ghostty, Warp and VS Code's terminal too. Today it reads Terminal and iTerm2, otherwise the clipboard.",
    why: "The command is only as good as the terminals it can see.",
    note: "One of the smaller items in the next phase.", source: "ROADMAP.md",
  },

  // ---- Context
  {
    id: "vocab", label: "Developer vocabulary", title: "Developer vocabulary",
    lane: 1, col: 0, status: "shipped",
    what: "An editable list of developer terms (plus one-click vocabulary books) that biases Whisper toward words like <code>kubectl</code> and is passed to the LLM so it doesn't “fix” them.",
    why: "General dictation mangles jargon.",
    source: "FEATURES.md",
  },
  {
    id: "codebase", label: "Codebase names", title: "Names from your codebase",
    lane: 1, col: 1, status: "shipped",
    what: "Finds the repo behind the front terminal or editor, reads its file, symbol, dependency and branch names, and turns spoken names into real ones: “the paste text function in the paste file” gives <code>paste_text</code> and <code>@src-tauri/src/paste.rs</code>.",
    why: "Talking to agents with real file and symbol names, locally, is what nobody else does.",
    note: "Shipped in 0.3.", source: "FEATURES.md",
  },
  {
    id: "corrections", label: "Learns from fixes", title: "Learns from your corrections",
    lane: 1, col: 1, status: "shipped",
    what: "History keeps the original next to refined text. Fix the same word twice and Settings offers to add it as a term and a replacement rule.",
    why: "The fastest vocabulary is the one you build without trying.",
    note: "Shipped in 0.3.", source: "FEATURES.md",
  },
  {
    id: "point", label: "Point at context", title: "Context you can point at",
    lane: 1, col: 3, status: "progress", order: 3,
    what: "“This” means the selected text, “look at this” attaches a screenshot of the front window, “attach the diff” adds your changes, and “line 42 of the paste file” becomes <code>@src-tauri/src/paste.rs:42</code>.",
    why: "Half of a good prompt is the context; you should be able to point at it instead of copying it.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "BACKLOG.md",
  },
  {
    id: "structured", label: "Structured prompts", title: "Structured prompts from long rambles",
    lane: 1, col: 4, status: "progress", order: 5,
    what: "Long dictations to a coding agent get a light structure (goal, context, constraints, done when), keeping every requirement and adding nothing.",
    why: "Thinking out loud is fast; agents work better with a tidy brief.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "BACKLOG.md",
  },
  {
    id: "repo-config", label: "Per-repo config", title: "Per-repo .mouthpiece.json",
    lane: 1, col: 4, status: "progress", order: 6,
    what: "A <code>.mouthpiece.json</code> committed to a repo with team vocabulary and spoken replacements, picked up whenever you work in that repo. Vocabulary applies on its own; replacements only once you allow that exact file. Your own settings win.",
    why: "Shares a team's terms, and every repo carrying the file tells whoever clones it about Mouthpiece.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "BACKLOG.md",
  },
  {
    id: "commit-msg", label: "Commit-message help", title: "Commit-message help",
    lane: 1, col: 4, status: "progress", order: 7,
    what: "Draft a commit message from the staged diff and what you say about it.",
    why: "A small, frequent chore that already has all its context in the repo.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "ROADMAP.md",
  },

  // ---- Dictation
  {
    id: "whisper", label: "Local whisper.cpp", title: "Local dictation with whisper.cpp",
    lane: 2, col: 0, status: "shipped",
    what: "Push-to-talk dictation with whisper.cpp on Metal. Tiny, Base and Small English models, verified downloads, warm-loaded at launch.",
    why: "Fast enough to feel instant, with nothing leaving the Mac.",
    source: "FEATURES.md",
  },
  {
    id: "commands", label: "Casing & commands", title: "Casing and voice commands",
    lane: 2, col: 0, status: "shipped",
    what: "“Snake case error response handler” gives <code>error_response_handler</code>. Also lists, backtrack (“actually”), spoken punctuation, snippets and boilerplate generation.",
    why: "Code-shaped output without an LLM round trip.",
    source: "FEATURES.md",
  },
  {
    id: "live", label: "Live transcription", title: "Transcribe while you talk",
    lane: 2, col: 1, status: "shipped",
    what: "Voice activity detection cuts audio at your pauses and transcribes each phrase in the background, with live drafts in the widget. Only the tail is left when you stop.",
    why: "Text is ready about a quarter of a second after you stop, however long you talked.",
    note: "Shipped in 0.3.", source: "FEATURES.md",
  },
  {
    id: "esc", label: "Esc pause & styles", title: "Esc pause and seven widget styles",
    lane: 2, col: 1, status: "shipped",
    what: "Esc pauses a recording with Resume, Transcribe and Delete, and is passed on to the app in front. Seven widget styles, from Minimal to Notch to Live caption.",
    why: "Interruptions happen mid-sentence; throwing the recording away shouldn't be the only option.",
    source: "FEATURES.md",
  },
  {
    id: "punctuation", label: "Spoken punctuation", title: "Spoken code punctuation",
    lane: 2, col: 4, status: "progress", order: 7,
    what: "Say “open paren”, “fat arrow”, “double colon” or “triple equals” and get <code>(</code>, <code>=&gt;</code>, <code>::</code>, <code>===</code>. Only phrases nobody says in ordinary prose.",
    why: "Some code is easier to say than to describe.",
    note: "Built on 2026-11-06; waiting for a hands-on check before release.", source: "ROADMAP.md",
  },
  {
    id: "parakeet", label: "Parakeet model", title: "Parakeet as an alternative speech model",
    lane: 2, col: 5, status: "research",
    what: "NVIDIA's Parakeet via <code>parakeet-rs</code>. A first spike on an M1 Pro found it about 2x slower than whisper.cpp on Metal and over 10x larger, with comparable accuracy on clean audio.",
    why: "It may still win on hard audio (noise, accents). The next step is a harder audio test, not integration.",
    note: "Research only.", source: "BACKLOG.md",
  },

  // ---- Modes & automation
  {
    id: "modes", label: "App-aware modes", title: "App-aware modes",
    lane: 3, col: 0, status: "shipped",
    what: "The app in front picks a mode: CLI for terminals, Messaging for Slack, Messages and Discord, Default elsewhere. Each mode has its own speech model, LLM and instructions.",
    why: "The same words should land differently in a terminal and in Slack.",
    source: "FEATURES.md",
  },
  {
    id: "history", label: "History & search", title: "History, search and reprocessing",
    lane: 3, col: 0, status: "shipped",
    what: "A local transcript history with full-text search, re-running a past dictation through another mode, Insights and an optional work journal.",
    why: "Nothing you said should be lost to a bad paste.",
    source: "FEATURES.md",
  },
  {
    id: "deeplinks", label: "Deep links & webhook", title: "Deep links and output webhook",
    lane: 3, col: 0, status: "shipped",
    what: "<code>mouthpiece://start-recording</code>, <code>stop-recording</code> and <code>toggle-recording</code> for Raycast, Alfred or scripts, and a JSON webhook after every dictation.",
    why: "Fits into automation you already have.",
    source: "FEATURES.md",
  },
  {
    id: "link-optout", label: "Link opt-out", title: "Opt-out for deep links starting a recording",
    lane: 3, col: 3, status: "planned",
    what: "A setting to stop links from starting the microphone at all, for people who never script it.",
    why: "Links already need this install's token; an off switch removes the path entirely.",
    note: "Production-readiness item 3.", source: "ROADMAP.md",
  },
  {
    id: "modes-ui", label: "Modes UI redesign", title: "Full Modes UI redesign",
    lane: 3, col: 5, status: "planned",
    what: "Named presets, per-mode shortcuts and a categorized app picker. The backend groundwork is in place; this is UI and data-model work.",
    why: "Modes are powerful but still awkward to set up.",
    note: "Later.", source: "ROADMAP.md",
  },
  {
    id: "config-agent", label: "Local Config Agent", title: "The Local Config Agent",
    lane: 3, col: 5, status: "planned",
    what: "Voice-driven edits to <code>~/.zshrc</code>, Neovim's <code>init.lua</code> or MCP settings.",
    why: "From the original pitch. It needs file write access driven by voice, so it needs a safety design first: a confirmation step and a diff preview.",
    note: "Deliberately deferred.", source: "ROADMAP.md",
  },

  // ---- Voice & meetings
  {
    id: "meeting1", label: "Meeting mode", title: "Meeting mode",
    lane: 4, col: 1, status: "shipped",
    what: "Records a meeting and writes a Markdown transcript split by speaker, made on your Mac. Your enrolled voice is “You”. Crash-safe, with a 2.5-hour cap.",
    why: "A local alternative to cloud notetakers.",
    source: "FEATURES.md",
  },
  {
    id: "meeting2", label: "Meetings phase 2", title: "Meeting mode, phase 2",
    lane: 4, col: 2, status: "testing",
    what: "System audio next to your mic on macOS 14.4+ (headphones just work), an echo gate for speakers, push-to-talk during a meeting, call detection prompts, and speaker names read from Meet, Slack Huddle, Teams and Zoom windows.",
    why: "Most meetings are calls, and “Speaker 2” isn't a name.",
    note: "Built 2026-11-05; awaiting a manual end-to-end check.", source: "ROADMAP.md",
  },
  {
    id: "isolated", label: "Isolated Voice", title: "Isolated Voice",
    lane: 4, col: 2, status: "testing",
    what: "Filters a recording down to your voice before Whisper sees it, using voice enrollment and speaker embeddings, so a second person talking is rejected, not just background noise.",
    why: "Open-plan offices and shared rooms.",
    note: "Both phases built; needs real enrollment and threshold tuning.", source: "ROADMAP.md",
  },
  {
    id: "always-on", label: "Always-on listening", title: "Always-on listening",
    lane: 4, col: 2, status: "testing",
    what: "Say “Mouthpiece, …” to dictate hands-free. Only your enrolled voice is acted on, it pastes only after the wake phrase, sleeps after 10 idle minutes and pauses during calls.",
    why: "Hands-free dictation without typing a side conversation into your editor.",
    note: "Built; frozen until manually verified.", source: "ROADMAP.md",
  },
  {
    id: "command-mode", label: "Command Mode", title: "Command Mode",
    lane: 4, col: 5, status: "planned",
    what: "A second, opt-in always-listening mode where short phrases trigger actions (keystrokes, shell commands, deep links) instead of typing text. Talon-inspired.",
    why: "Some things you want to do, not say.",
    note: "Later. Bigger than everything else on this page combined.", source: "ROADMAP.md",
  },
  {
    id: "duplex", label: "Voice conversation", title: "Conversational voice loop",
    lane: 4, col: 5, status: "research",
    what: "Continuous, full-duplex voice with local text-to-speech, and a Socratic “rubber duck” mode that asks questions instead of writing code.",
    why: "The long-term vision from the product spec.",
    note: "Nothing started; everything else is groundwork.", source: "PRODUCT_SPEC.md",
  },

  // ---- Trust & local AI
  {
    id: "llm", label: "Local LLM cleanup", title: "Local LLM cleanup",
    lane: 5, col: 0, status: "shipped",
    what: "Optional refinement through Ollama, only when the transcript needs it. Cloud models are never used.",
    why: "Cleans up rambling speech without sending it anywhere.",
    source: "FEATURES.md",
  },
  {
    id: "local-servers", label: "More local LLMs", title: "LM Studio, llama.cpp, Jan and .gguf import",
    lane: 5, col: 1, status: "shipped",
    what: "Models from LM Studio, llama.cpp's server and Jan are listed next to Ollama's, loopback only. A .gguf file on disk can be added to Ollama without downloading anything.",
    why: "For Macs where Ollama's downloads are blocked but a model is already there.",
    source: "FEATURES.md",
  },
  {
    id: "privacy", label: "History lock & tokens", title: "History lock and token-protected links",
    lane: 5, col: 1, status: "shipped",
    what: "Touch ID or password before History is shown, transcript text kept out of logs, and a per-install token on every <code>mouthpiece://</code> link and the agent hook.",
    why: "Any web page can open a <code>mouthpiece://</code> link; without the token it could start your mic.",
    source: "FEATURES.md",
  },
  {
    id: "hardened", label: "Hardened runtime", title: "Hardened runtime on release builds",
    lane: 5, col: 2, status: "testing",
    what: "Release builds are signed with macOS's hardened runtime, so other code can't inject into Mouthpiece and use its microphone or Accessibility grants.",
    why: "An app that hears you and types for you is worth protecting.",
    note: "Needs checking in a real signed build.", source: "ROADMAP.md",
  },
  {
    id: "mic-perm", label: "Mic permission check", title: "Microphone permission in the setup checklist",
    lane: 5, col: 1, status: "shipped",
    what: "The setup checklist shows the microphone permission state, with one click to ask or to open the right System Settings page, and names a device-management block as such.",
    why: "A missing permission used to mean a mic that silently recorded nothing.",
    note: "Shipped in 0.3.4, together with the missing microphone usage description that caused it.", source: "FEATURES.md",
  },
  {
    id: "notarize", label: "Notarization", title: "Notarization",
    lane: 5, col: 3, status: "planned",
    what: "Notarize releases with an Apple Developer account, so the first launch doesn't need System Settings › Open Anyway.",
    why: "The biggest bump in installing today.",
    note: "Production-readiness item 2. Needs a paid Apple Developer account.", source: "ROADMAP.md",
  },
  {
    id: "codesign", label: "Stable codesigning", title: "Stable local codesigning identity",
    lane: 5, col: 4, status: "planned",
    what: "Sign development builds with one consistent self-signed certificate, so macOS stops revoking the Accessibility grant on every rebuild.",
    why: "A changed signature has silently stopped pasting before.",
    note: "Touches Keychain trust settings, so it's done live rather than unsupervised.", source: "ROADMAP.md",
  },
];

// [from, to]: "to" builds on "from".
const EDGES = [
  ["vocab", "codebase"],
  ["vocab", "corrections"],
  ["codebase", "agent-commands"],
  ["agent-commands", "reply"],
  ["reply", "fleet"],
  ["fleet", "mcp"],
  ["reply", "more-hooks"],
  ["agent-commands", "more-errors"],
  ["codebase", "point"],
  ["point", "structured"],
  ["point", "commit-msg"],
  ["codebase", "repo-config"],
  ["corrections", "repo-config"],
  ["history", "corrections"],
  ["whisper", "commands"],
  ["whisper", "live"],
  ["whisper", "parakeet"],
  ["commands", "punctuation"],
  ["modes", "modes-ui"],
  ["modes", "repo-config"],
  ["deeplinks", "privacy"],
  ["deeplinks", "mcp"],
  ["privacy", "link-optout"],
  ["meeting1", "meeting2"],
  ["isolated", "always-on"],
  ["always-on", "command-mode"],
  ["always-on", "duplex"],
  ["llm", "local-servers"],
  ["local-servers", "config-agent"],
  ["hardened", "notarize"],
  ["mic-perm", "notarize"],
  ["notarize", "codesign"],
];

// ---------------------------------------------------------------- render

(() => {
const SVG_NS = "http://www.w3.org/2000/svg";
const REPO = "https://github.com/JFinchy/mouthpiece/blob/main/";
const byId = new Map(NODES.map((n) => [n.id, n]));
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

const graph = document.querySelector("[data-graph]");
const panel = document.querySelector("[data-panel]");
const scrim = document.querySelector("[data-scrim]");
const hidden = new Set();
let layoutMode = null;
let lastWidth = 0;
let openId = null;
let returnFocus = null;
let firstDraw = true;

function el(name, attrs = {}, parent) {
  const node = document.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  if (parent) parent.append(node);
  return node;
}

// The status marker: shape and fill carry the status, not just the color.
function mark(status, cx, cy, parent, r = 4.5) {
  const g = el("g", { class: "node-mark", "aria-hidden": "true" }, parent);
  const color = `var(--st-${status})`;
  if (status === "shipped") {
    el("circle", { cx, cy, r, fill: color }, g);
  } else if (status === "testing") {
    el("circle", { cx, cy, r, fill: "none", stroke: color, "stroke-width": 1.6 }, g);
    el("path", { d: `M${cx} ${cy - r} A${r} ${r} 0 0 0 ${cx} ${cy + r} Z`, fill: color }, g);
  } else if (status === "progress") {
    el("circle", { cx, cy, r, fill: "none", stroke: color, "stroke-width": 1.6 }, g);
    el("circle", { cx, cy, r: r * 0.45, fill: color }, g);
  } else if (status === "planned") {
    el("circle", { cx, cy, r, fill: "none", stroke: color, "stroke-width": 1.6 }, g);
  } else {
    const d = r + 0.8;
    el("path", { d: `M${cx} ${cy - d} L${cx + d} ${cy} L${cx} ${cy + d} L${cx - d} ${cy} Z`, fill: "none", stroke: color, "stroke-width": 1.6 }, g);
  }
  return g;
}

function markSvg(status, size = 14) {
  const svg = el("svg", { width: size, height: size, viewBox: `0 0 ${size} ${size}`, "aria-hidden": "true" });
  mark(status, size / 2, size / 2, svg, size / 2 - 2.5);
  return svg;
}

// Positions for the wide layout: columns are stages, row bands are lanes.
function layoutWide() {
  const colW = 188, pillW = 170, pillH = 30, rowGap = 10, left = 18, head = 70, laneTop = 22, lanePad = 16;
  const cells = new Map();
  for (const n of NODES) {
    const key = `${n.lane}:${n.col}`;
    if (!cells.has(key)) cells.set(key, []);
    cells.get(key).push(n);
  }
  const laneRows = LANES.map((_, lane) =>
    Math.max(1, ...STAGES.map((_, col) => (cells.get(`${lane}:${col}`) || []).length)),
  );
  const laneY = [];
  let y = head;
  laneRows.forEach((rows, i) => {
    laneY[i] = y;
    y += laneTop + rows * (pillH + rowGap) - rowGap + lanePad + 10;
  });
  const pos = new Map();
  for (const [key, list] of cells) {
    const [lane, col] = key.split(":").map(Number);
    list.forEach((n, i) => {
      pos.set(n.id, { x: left + col * colW, y: laneY[lane] + laneTop + i * (pillH + rowGap), w: pillW, h: pillH });
    });
  }
  return { width: left + STAGES.length * colW, height: y + 4, pos, laneY, colW, left, head };
}

// Positions for the narrow layout: one node per row, grouped by stage,
// with each lane as a vertical track on the left like a transit map.
function layoutNarrow(width) {
  const trackX = (lane) => 16 + lane * 12;
  const pillX = trackX(LANES.length - 1) + 22;
  const pillW = Math.min(width - pillX - 10, 440);
  const pillH = 42, rowH = 50, stageH = 54;
  const pos = new Map();
  const stageY = [];
  let y = 10;
  STAGES.forEach((_, col) => {
    const list = NODES.filter((n) => n.col === col).sort((a, b) => a.lane - b.lane);
    if (!list.length) return;
    stageY[col] = y;
    y += stageH;
    list.forEach((n) => {
      pos.set(n.id, { x: pillX, y, w: pillW, h: pillH, dot: trackX(n.lane) });
      y += rowH;
    });
    y += 8;
  });
  return { width, height: y + 6, pos, stageY, trackX, pillX };
}

function edgePath(a, b, mode) {
  if (mode === "narrow") {
    const x1 = a.dot, y1 = a.y + a.h / 2, x2 = b.dot, y2 = b.y + b.h / 2;
    const dy = (y2 - y1) / 2;
    return `M${x1} ${y1} C${x1} ${y1 + dy} ${x2} ${y2 - dy} ${x2} ${y2}`;
  }
  if (Math.abs(a.x - b.x) < 1) {
    // Same column: a bracket out to the left, so it never crosses a pill.
    const x = a.x, y1 = a.y + a.h / 2, y2 = b.y + b.h / 2;
    const out = 10 + Math.min(10, Math.abs(y2 - y1) / 30);
    return `M${x} ${y1} C${x - out} ${y1} ${x - out} ${y2} ${x} ${y2}`;
  }
  const x1 = a.x + a.w, y1 = a.y + a.h / 2, x2 = b.x, y2 = b.y + b.h / 2;
  const dx = Math.max(24, (x2 - x1) / 2);
  return `M${x1} ${y1} C${x1 + dx} ${y1} ${x2 - dx} ${y2} ${x2} ${y2}`;
}

function neighbors(id) {
  const out = [];
  for (const [from, to] of EDGES) {
    if (from === id) out.push({ id: to, kind: "leads to" });
    if (to === id) out.push({ id: from, kind: "builds on" });
  }
  return out;
}

function render() {
  const width = graph.clientWidth;
  const mode = width < 940 ? "narrow" : "wide";
  if (mode === layoutMode && (mode === "wide" || Math.abs(width - lastWidth) < 2)) return;
  layoutMode = mode;
  lastWidth = width;
  const focusedId = document.activeElement?.closest?.(".node")?.dataset.id;

  const L = mode === "wide" ? layoutWide() : layoutNarrow(width);
  graph.textContent = "";
  const svg = el("svg", {
    viewBox: `0 0 ${L.width} ${L.height}`,
    role: "group",
    "aria-label": "Roadmap graph. Each item is a button that opens its details.",
  }, graph);
  if (mode === "narrow") svg.style.height = L.height + "px";

  const back = el("g", {}, svg);
  if (mode === "wide") {
    // Column bands and headings, a "today" line, lane labels.
    STAGES.forEach((s, col) => {
      const x = L.left + col * L.colW - 9;
      if (col % 2 === 1) el("rect", { class: "col-band", x, y: 0, width: L.colW, height: L.height }, back);
      el("text", { class: "col-head", x: x + 9, y: 28 }, back).textContent = s.head;
      el("text", { class: "col-sub", x: x + 9, y: 48 }, back).textContent = s.sub;
    });
    const todayX = L.left + 3 * L.colW - 9;
    el("line", { x1: todayX, y1: 58, x2: todayX, y2: L.height, stroke: "var(--amber)", "stroke-width": 1, "stroke-dasharray": "3 4", opacity: 0.7 }, back);
    el("text", { class: "col-head", x: todayX - 6, y: 66, "text-anchor": "end", fill: "var(--amber)", style: "fill: var(--amber)" }, back).textContent = "Today";
    LANES.forEach((name, lane) => {
      if (lane > 0) el("line", { class: "lane-rule", x1: 0, x2: L.width, y1: L.laneY[lane] - 6, y2: L.laneY[lane] - 6 }, back);
      el("text", { class: "lane-label", x: L.left, y: L.laneY[lane] + 12 }, back).textContent = name;
    });
  } else {
    LANES.forEach((_, lane) => {
      el("line", { class: "track", x1: L.trackX(lane), x2: L.trackX(lane), y1: 0, y2: L.height }, back);
    });
    STAGES.forEach((s, col) => {
      if (L.stageY[col] === undefined) return;
      el("text", { class: "col-head", x: L.pillX, y: L.stageY[col] + 22 }, back).textContent = s.head;
      el("text", { class: "col-sub", x: L.pillX, y: L.stageY[col] + 41 }, back).textContent = s.sub;
    });
  }

  const edgeLayer = el("g", {}, svg);
  const nodeLayer = el("g", {}, svg);

  EDGES.forEach(([from, to], i) => {
    const a = L.pos.get(from), b = L.pos.get(to);
    if (!a || !b) return;
    const target = byId.get(to);
    const path = el("path", {
      class: "edge" + (target.status === "planned" || target.status === "research" ? " to-planned" : ""),
      d: edgePath(a, b, mode),
      "data-from": from,
      "data-to": to,
    }, edgeLayer);
    if (firstDraw && !reduceMotion) {
      path.classList.add("draw");
      path.style.setProperty("--delay", 250 + i * 18 + "ms");
    }
  });

  NODES.forEach((n, i) => {
    const p = L.pos.get(n.id);
    const status = STATUS[n.status].label;
    const g = el("g", {
      class: "node",
      tabindex: "0",
      role: "button",
      "data-id": n.id,
      "data-status": n.status,
      "aria-label": `${n.title}. ${status}. ${STAGES[n.col].head}, ${STAGES[n.col].sub}. ${LANES[n.lane]}.`,
      "aria-haspopup": "dialog",
    }, nodeLayer);
    if (firstDraw && !reduceMotion) {
      g.classList.add("draw");
      g.style.setProperty("--delay", n.col * 70 + n.lane * 25 + "ms");
    }
    if (mode === "narrow") {
      el("line", { class: "tick", x1: p.dot, x2: p.x, y1: p.y + p.h / 2, y2: p.y + p.h / 2 }, g);
      el("circle", { cx: p.dot, cy: p.y + p.h / 2, r: 3.5, fill: `var(--st-${n.status})`, class: "node-mark" }, g);
    }
    el("rect", { class: "focus-ring", x: p.x - 3, y: p.y - 3, width: p.w + 6, height: p.h + 6, rx: 11 }, g);
    el("rect", { class: "node-box", x: p.x, y: p.y, width: p.w, height: p.h, rx: 8 }, g);
    const mx = p.x + 14, my = p.y + (mode === "narrow" ? 15 : p.h / 2);
    if (n.status === "testing") el("circle", { class: "pulse-ring", cx: mx, cy: my, r: 4.5 }, g);
    mark(n.status, mx, my, g);
    const label = el("text", { class: "node-label", x: p.x + 26, y: my + 4.2 }, g);
    label.textContent = mode === "narrow" ? n.title : n.label;
    if (mode === "narrow") {
      // Keep long titles inside the pill.
      const max = Math.floor((p.w - 36) / 6.6);
      if (n.title.length > max) label.textContent = n.label;
      el("text", { class: "lane-label", x: p.x + 26, y: p.y + 32 }, g).textContent = `${LANES[n.lane]} · ${status}`;
    }
    // Priority number, only where the label leaves room for it.
    if (n.order && label.getComputedTextLength() + 26 + 30 <= p.w) {
      el("text", { class: "lane-label", x: p.x + p.w - 10, y: my + 3.5, "text-anchor": "end", "aria-hidden": "true" }, g).textContent = `#${n.order}`;
    }
  });

  firstDraw = false;
  applyFilter();
  if (openId) graph.querySelector(`.node[data-id="${openId}"]`)?.classList.add("active");
  if (focusedId) graph.querySelector(`.node[data-id="${focusedId}"]`)?.focus();
}

// ---------------------------------------------------------------- interaction

function highlight(id) {
  const svg = graph.querySelector("svg");
  if (!svg) return;
  graph.classList.toggle("dimming", Boolean(id));
  svg.querySelectorAll(".lit").forEach((x) => x.classList.remove("lit"));
  if (!id) return;
  const ids = new Set([id, ...neighbors(id).map((x) => x.id)]);
  svg.querySelectorAll(".node").forEach((g) => g.classList.toggle("lit", ids.has(g.dataset.id)));
  svg.querySelectorAll(".edge").forEach((p) => p.classList.toggle("lit", p.dataset.from === id || p.dataset.to === id));
}

function applyFilter() {
  graph.querySelectorAll(".node").forEach((g) => {
    const off = hidden.has(g.dataset.status);
    g.classList.toggle("filtered", off);
    g.setAttribute("tabindex", off ? "-1" : "0");
    g.setAttribute("aria-hidden", off ? "true" : "false");
  });
  graph.querySelectorAll(".edge").forEach((p) => {
    const off = hidden.has(byId.get(p.dataset.from).status) || hidden.has(byId.get(p.dataset.to).status);
    p.classList.toggle("filtered", off);
  });
}

function openPanel(id, { focusPanel = true } = {}) {
  const n = byId.get(id);
  if (!n) return;
  if (!openId) returnFocus = document.activeElement;
  openId = id;
  graph.querySelectorAll(".node.active").forEach((g) => g.classList.remove("active"));
  graph.querySelector(`.node[data-id="${id}"]`)?.classList.add("active");

  const tag = panel.querySelector("[data-p-status]");
  tag.className = `tag tag-${n.status}`;
  tag.textContent = STATUS[n.status].label;
  panel.querySelector("[data-p-stage]").textContent = `${STAGES[n.col].head} · ${STAGES[n.col].sub} · ${LANES[n.lane]}`;
  panel.querySelector("[data-p-title]").textContent = n.title;
  panel.querySelector("[data-p-what]").innerHTML = n.what;
  panel.querySelector("[data-p-why]").innerHTML = n.why;
  const noteWrap = panel.querySelector("[data-p-note-wrap]");
  noteWrap.hidden = !n.note && !n.order;
  panel.querySelector("[data-p-note]").textContent = n.note || "";
  const src = panel.querySelector("[data-p-source]");
  src.href = REPO + n.source;
  src.textContent = `Read more in ${n.source}`;

  const list = panel.querySelector("[data-p-related]");
  list.textContent = "";
  const rel = neighbors(id);
  panel.querySelector("[data-p-related-wrap]").hidden = rel.length === 0;
  for (const r of rel) {
    const other = byId.get(r.id);
    const li = document.createElement("li");
    const b = document.createElement("button");
    b.type = "button";
    b.append(markSvg(other.status));
    b.append(document.createTextNode(other.title));
    const kind = document.createElement("span");
    kind.className = "rel-kind";
    kind.textContent = `${r.kind} · ${STATUS[other.status].label.toLowerCase()}`;
    b.append(kind);
    b.addEventListener("click", () => openPanel(r.id));
    li.append(b);
    list.append(li);
  }

  panel.classList.add("open");
  panel.setAttribute("aria-hidden", "false");
  scrim.classList.add("open");
  highlight(id);
  if (history.replaceState) history.replaceState(null, "", "#" + id);
  if (focusPanel) panel.querySelector("[data-p-title]").focus({ preventScroll: true });
}

function closePanel() {
  if (!openId) return;
  const id = openId;
  openId = null;
  panel.classList.remove("open");
  panel.setAttribute("aria-hidden", "true");
  scrim.classList.remove("open");
  graph.querySelectorAll(".node.active").forEach((g) => g.classList.remove("active"));
  highlight(null);
  if (history.replaceState) history.replaceState(null, "", location.pathname + location.search);
  const back = returnFocus && document.contains(returnFocus) ? returnFocus : graph.querySelector(`.node[data-id="${id}"]`);
  back?.focus({ preventScroll: false });
  returnFocus = null;
}

function visibleNodes() {
  return [...graph.querySelectorAll(".node")].filter((g) => !g.classList.contains("filtered"));
}

if (graph && panel) {
  render();
  let resizeTimer;
  window.addEventListener("resize", () => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(render, 120);
  });

  graph.addEventListener("click", (e) => {
    const g = e.target.closest(".node");
    if (g && !g.classList.contains("filtered")) openPanel(g.dataset.id);
  });
  graph.addEventListener("keydown", (e) => {
    const g = e.target.closest(".node");
    if (!g) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      openPanel(g.dataset.id);
      return;
    }
    const moves = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
    if (e.key in moves) {
      e.preventDefault();
      const list = visibleNodes();
      const i = list.indexOf(g);
      list[(i + moves[e.key] + list.length) % list.length]?.focus();
    }
  });
  graph.addEventListener("mouseover", (e) => {
    const g = e.target.closest(".node");
    if (!openId) highlight(g && !g.classList.contains("filtered") ? g.dataset.id : null);
  });
  graph.addEventListener("mouseleave", () => { if (!openId) highlight(null); });
  graph.addEventListener("focusin", (e) => {
    const g = e.target.closest(".node");
    if (g && !openId) highlight(g.dataset.id);
  });
  graph.addEventListener("focusout", () => { if (!openId) highlight(null); });

  panel.querySelector("[data-p-close]").addEventListener("click", closePanel);
  scrim.addEventListener("click", closePanel);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && openId) {
      e.preventDefault();
      closePanel();
    }
  });

  // Status filters in the legend.
  document.querySelectorAll("[data-filter]").forEach((b) => {
    const status = b.dataset.filter;
    b.prepend(markSvg(status));
    const count = b.querySelector(".lg-count");
    if (count) count.textContent = NODES.filter((n) => n.status === status).length;
    b.addEventListener("click", () => {
      const on = b.getAttribute("aria-pressed") !== "false";
      b.setAttribute("aria-pressed", String(!on));
      if (on) hidden.add(status); else hidden.delete(status);
      applyFilter();
    });
  });

  // Text lists under the graph open the same panel.
  document.querySelectorAll("[data-open]").forEach((b) => {
    b.addEventListener("click", () => openPanel(b.dataset.open));
  });

  const openFromHash = () => {
    const id = location.hash.slice(1);
    if (byId.has(id)) openPanel(id, { focusPanel: false });
  };
  openFromHash();
  window.addEventListener("hashchange", openFromHash);
}
})();
