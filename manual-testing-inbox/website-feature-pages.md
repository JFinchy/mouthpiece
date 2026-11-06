# Website: Features, Guides and Roadmap pages

Branch: `worktree-agent-abda12d10d6b64e24`
Files changed: `site/index.html`, `site/main.js`, `site/styles.css`, new
`site/features.html`, `site/guides.html`, `site/roadmap.html`,
`site/roadmap.js`

## What to verify

The site gets three new pages beside the download page: Features (what
Mouthpiece does, with demos), Guides (setup walk-throughs with
copyable snippets) and Roadmap (an interactive graph built from
`site/roadmap.js`). Playwright checked all four pages in dark and light themes
at 1280px and 375px for console errors, sideways scrolling and broken
anchors. A person still needs to judge how the pages read and look on real
devices, check that the copy buttons put the right text on a real clipboard,
and see how the roadmap graph feels under a mouse, keyboard and touch.

## Steps

1. Serve the site locally:
   `cd site && python3 -m http.server 8000 --bind 127.0.0.1`, then open
   <http://127.0.0.1:8000/>. (After merge, the GitHub Pages deploy from
   `.github/workflows/pages.yml` serves the same files.)
2. On `index.html`, the header shows Features, Guides, Roadmap, GitHub and
   Download. The "Next up: a voice layer for a fleet of agents" badge
   opens the Roadmap.
3. **Features** (`features.html`): scroll every section (Codebase, Agents,
   Code, Dictation, Meetings, Local AI, Privacy, Automation). In the Agents
   demo, the rustc error lines stay in columns; at phone width the
   `-->`/`212 |` lines scroll sideways, and the error and help lines
   wrap. The Dictation demo shows four widget stages and seven styles.
4. **Guides** (`guides.html`): click a table-of-contents link. The page
   jumps there, and the current section stays highlighted in the
   contents as you scroll. In "Claude Code", click **Copy** and paste
   into a text editor: you get the full hooks JSON, and the button briefly
   reads "Copied".
5. **Roadmap** (`roadmap.html`):
   - "Several agents at once" and "CLI + MCP server" are pink with an
     "In progress" tag, and they're listed under "Being built now".
     "Mic permission check" sits in the Shipped · Agents era column.
   - Click a node: a panel opens with What it is / Why / Status /
     Connected to, and the URL gets `#<id>`. Click a related item, then
     press Esc. The panel closes and focus goes back to the node.
   - Tab to a node, use the arrow keys to move between nodes, and press
     Enter to open one.
   - Click "Shipped" in the legend. Shipped nodes fade out. Click it again
     to bring them back.
   - Open `roadmap.html#meeting2` directly: the panel opens on Meeting
     mode, phase 2.
6. Repeat steps 3-5 at phone width: either a real phone, or
   Safari/Chrome responsive mode at 375px. Use **Menu** to navigate. No
   page scrolls sideways, and the roadmap shows as a vertical list.
7. Switch macOS between Light and Dark (System Settings › Appearance) and
   reload each page.

## Pass criteria

All four pages render in both themes on desktop and phone with no
sideways page scroll, clipped text or unreadable colors. Copy buttons put
the exact snippet on the clipboard. The roadmap panel, keyboard
navigation, legend filters and `#id` links all work, and the statuses
match `ROADMAP.md`/`FEATURES.md`.
