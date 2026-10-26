// Marks that scripts run, so scroll reveals only hide content when
// something will bring it back.
document.documentElement.classList.add("js");

const revealed = document.querySelectorAll(".reveal");
if ("IntersectionObserver" in window) {
  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          entry.target.classList.add("in");
          io.unobserve(entry.target);
        }
      }
    },
    { rootMargin: "0px 0px -8% 0px" },
  );
  revealed.forEach((el) => io.observe(el));
} else {
  revealed.forEach((el) => el.classList.add("in"));
}

// Copy buttons next to commands.
document.querySelectorAll("[data-copy]").forEach((button) => {
  button.addEventListener("click", () => {
    const text = button.parentElement.querySelector("[data-cmd]")?.textContent?.trim();
    if (!text || !navigator.clipboard) return;
    navigator.clipboard.writeText(text).then(() => {
      button.textContent = "Copied";
      button.dataset.copied = "";
      setTimeout(() => {
        button.textContent = "Copy";
        delete button.dataset.copied;
      }, 1600);
    }, () => {});
  });
});

// Fill in the latest release version, quietly keeping the fallback text
// if GitHub's API is unreachable or rate-limited.
fetch("https://api.github.com/repos/JFinchy/mouthpiece/releases/latest")
  .then((r) => (r.ok ? r.json() : null))
  .then((release) => {
    if (!release || !release.tag_name) return;
    const version = "Version " + release.tag_name.replace(/^v/, "");
    document.querySelectorAll("[data-version]").forEach((el) => (el.textContent = version));
  })
  .catch(() => {});

// Hero demo: three dictations into three apps, on a loop. Each one walks
// through the widget's real stages: listening, transcribing (amber),
// refining (violet, only when the transcript needs it), then the paste.
const scenes = [
  {
    title: "Terminal — agent",
    spoken: "Look at recording dot rs and fix the crash, press enter.",
    refine: false,
    skipNote: "Clean, skipped refining",
    body: (typed) => `
      <div class="dim">✓ Reproduced: recording stops after 30s, then the app exits.</div>
      <div>Where should I look first?</div>
      <div class="prompt">${typed}</div>`,
    output: [
      { text: "Look at " },
      { text: "@recording.rs", cls: "tag" },
      { text: " and fix the crash" },
    ],
    after: `<div class="ok">↵ sent</div>`,
  },
  {
    title: "Cursor — profile.ts",
    spoken: "Camel case user profile ID.",
    refine: false,
    skipNote: "Casing command",
    body: (typed) => `
      <div><span class="kw">export async function</span> loadProfile(id) {</div>
      <div>&nbsp;&nbsp;<span class="kw">const</span> ${typed} = <span class="kw">await</span> getUser(id);</div>
      <div>&nbsp;&nbsp;<span class="kw">return</span> <span class="str">\`/u/\${id}\`</span>;</div>
      <div>}</div>`,
    output: [{ text: "userProfileId", cls: "typed" }],
  },
  {
    title: "Slack — #release",
    spoken: "So um I think we should uh ship it tonight, actually tomorrow.",
    refine: true,
    body: (typed) => `
      <div class="chat-name">Sam</div>
      <div class="chat-msg">When do you want to cut 0.3?</div>
      <div class="composer">${typed}</div>`,
    output: [{ text: "I think we should ship it tomorrow." }],
  },
  {
    // Replying to a Claude Code session that's waiting, from another app.
    title: "Terminal — api · claude",
    waiting: "Claude is waiting · api",
    spoken: "Yes, staging first, then show me the diff.",
    refine: false,
    skipNote: "Replying to api",
    body: (typed) => `
      <div class="dim">● Wrote migrations/0042_add_team_role.sql</div>
      <div class="ask">Run the migration on staging first, or straight to prod?</div>
      <div class="prompt">${typed}</div>`,
    output: [{ text: "Yes, staging first, then show me the diff." }],
    after: `<div class="ok">✓ Sent to api in Terminal</div>`,
  },
];

const $ = (sel) => document.querySelector(sel);
const body = $("[data-demo-body]");
const title = $("[data-demo-title]");
const widget = $("[data-widget]");
const label = $("[data-widget-label]");
const fill = $("[data-widget-fill]");
const spoken = $("[data-spoken]");
const meterBars = widget ? [...widget.querySelectorAll(".widget-meter i")] : [];

const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
let visible = true;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Holds the loop while the demo is scrolled away or the tab is hidden.
const waitVisible = async () => {
  while (!visible || document.hidden) await sleep(250);
};

const escapeHtml = (s) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function renderTyped(output, chars, caret) {
  let left = chars;
  let html = "";
  for (const part of output) {
    if (left <= 0) break;
    const piece = escapeHtml(part.text.slice(0, left));
    left -= part.text.length;
    html += part.cls ? `<span class="${part.cls}">${piece}</span>` : piece;
  }
  return html + (caret ? '<span class="caret"></span>' : "");
}

function setStage(stage, text, percent) {
  widget.dataset.stage = stage;
  label.textContent = text;
  fill.style.width = (percent ?? 0) + "%";
}

function setMeter(active) {
  meterBars.forEach((bar) => {
    bar.style.height = active ? 3 + Math.round(Math.random() * 11) + "px" : "3px";
  });
}

async function progress(stage, name, ms) {
  const start = performance.now();
  for (;;) {
    await waitVisible();
    const t = Math.min(1, (performance.now() - start) / ms);
    const pct = Math.round(t * 100);
    setStage(stage, `${name} ${pct}%`, pct);
    if (t >= 1) return;
    await sleep(40);
  }
}

async function playScene(scene) {
  const totalChars = scene.output.reduce((n, p) => n + p.text.length, 0);
  title.textContent = scene.title;
  body.innerHTML = scene.body(renderTyped(scene.output, 0, true));
  spoken.innerHTML = "&nbsp;";
  setStage("ready", "Ready", 0);
  await sleep(700);
  if (scene.waiting) {
    setStage("waiting", scene.waiting, 0);
    await sleep(1600);
    setStage("waiting", "⌃⌥R  Reply by voice", 0);
    await sleep(900);
  }

  // Listening: words arrive as they're said.
  const words = scene.spoken.split(" ");
  const started = performance.now();
  for (let i = 1; i <= words.length; i++) {
    await waitVisible();
    const secs = Math.floor((performance.now() - started) / 1000);
    setStage("listening", `Listening 0:0${Math.min(secs, 9)}`, 0);
    setMeter(true);
    spoken.textContent = "“" + words.slice(0, i).join(" ") + (i === words.length ? "”" : "");
    await sleep(170 + Math.random() * 120);
  }
  setMeter(false);
  await sleep(250);

  await progress("transcribing", "Transcribing", 650);
  if (scene.refine) {
    await progress("refining", "Refining", 900);
  } else {
    setStage("transcribing", scene.skipNote, 100);
    await sleep(500);
  }

  // The paste lands all at once in the real app; a quick type-in reads
  // better here and still feels instant.
  setStage("done", "Pasted", 100);
  for (let c = 1; c <= totalChars; c += 2) {
    await waitVisible();
    body.innerHTML = scene.body(renderTyped(scene.output, c, true));
    await sleep(16);
  }
  body.innerHTML = scene.body(renderTyped(scene.output, totalChars, false)) + (scene.after || "");
  await sleep(2600);
}

function renderStatic() {
  const scene = scenes[0];
  const total = scene.output.reduce((n, p) => n + p.text.length, 0);
  title.textContent = scene.title;
  body.innerHTML = scene.body(renderTyped(scene.output, total, false)) + scene.after;
  spoken.textContent = "“" + scene.spoken + "”";
  setStage("done", "Pasted", 100);
}

if (body && widget) {
  if ("IntersectionObserver" in window) {
    new IntersectionObserver((entries) => {
      visible = entries[0].isIntersecting;
    }).observe(body);
  }
  if (reduceMotion) {
    renderStatic();
  } else {
    (async () => {
      for (let i = 0; ; i = (i + 1) % scenes.length) await playScene(scenes[i]);
    })();
  }
}
