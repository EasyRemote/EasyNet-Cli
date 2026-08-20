---
name: easynet-pages-author
description: Write a small frontend (HTML/CSS/JS) plus a tiny declarative backend, then ship it as a real URL on this machine via `easynet pages`. Use when the user asks for a website, a product page, a small web app, or a "demo I can click on". The site runs through EasyNet's Pages reference system — the bytes are served by a kernel-sandboxed daemon, the backend is one TOML file per endpoint, and the deploy is a single `easynet pages create` call.
allowed-tools: [Bash, Write, Edit, Read]
---

# EasyNet Pages Author

You write a website by writing files on disk, then deploy it by calling `easynet pages create`. The site appears at `https://<project>.<user>.pages.<realm>/` (or `http://<project>.<user>.pages.localhost:<port>/` in dev). Two surfaces:

- **Static bytes** — `index.html`, `style.css`, `assets/*`, anything else under the project root. Served verbatim by `<user>.<project>.page.fetch` through a Linux `openat2 + RESOLVE_BENEATH` sandbox so paths cannot escape the published folder.
- **Dynamic backend** — TOML manifests under `<project>/api/<verb>.toml`. Each one becomes a real HTTP endpoint at `/api/<verb>`. v0 supports two manifest kinds: `static_json` (return a fixed JSON) and `echo` (return the request body merged with `extra`). Anything more sophisticated lives in a hand-written ability — out of scope for this skill.

## When this skill activates

User-prompted triggers:

- "Make me a website / landing page / product page / mini shop / demo site."
- "Build a frontend that calls a backend that does X." — even tiny X.
- "Show me how this would look live." — generate + deploy is faster than rendering ASCII.
- "Add an `/api/<verb>` to this project."

Self-prompted triggers — activate this skill yourself when YOU notice:

- The user described a UI but you're about to dump it as a code block instead of running it.
- You wrote `index.html` to `/tmp/` without telling the user how to view it. Deploy it instead — they get a URL.
- The user wants to "test" the frontend against a backend you haven't written. Write the `api/<verb>.toml` and deploy together.

## Process

### 1. Pick a project_id

The project_id ends up as the leftmost subdomain (`<project>.<user>.pages.<realm>/`). Use a short, URL-safe slug: `[a-zA-Z0-9_-]+`, max 64 chars, no dots. Examples: `shop`, `papers`, `dashboard-v2`. If the user did not name it, pick something descriptive of the content, not "site1".

### 2. Lay out the project folder

Pick `~/.easynet/web-apps/<project_id>/` as the canonical location (creates per-user, agent-independent). Only ONE rule on the layout: `device.easynet.app.toml` is reserved for EasyNet to read; the rest is yours.

```
~/.easynet/web-apps/<project_id>/
├── index.html              ← entry point (Hub maps `/` → `/index.html`)
├── style.css               ← global stylesheet
├── app.js                  ← optional client-side logic
├── assets/                 ← images, fonts, anything static
└── api/                    ← optional dynamic backend
    ├── list_items.toml
    ├── checkout.toml
    └── ...
```

`assets/` filenames may contain hash suffixes (e.g. `index-abc123.js`) when bundled by Vite/Webpack — the Hub serves them with the right `Content-Type` from a built-in MIME allow-list. Files outside the allow-list serve as `application/octet-stream` with `Content-Disposition: attachment` (safe default — browser downloads instead of executing).

### 3. Write the frontend

Plain HTML/CSS/JS works directly. There is no build step required for this skill. If you want to use a framework (React/Vue/Svelte), build it locally and copy the contents of `dist/` into the project folder — the Hub does not run a bundler for you.

The frontend can fetch its own backend through the relative URL `/api/<verb>` (same origin). No CORS configuration needed on your side; the Hub already sends `Access-Control-Allow-Origin: *` for `/api/*` responses. Example fetch:

```js
fetch('/api/list_items')
  .then(r => r.json())
  .then(items => render(items));

fetch('/api/checkout', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ item_id, qty, address }),
})
  .then(r => r.json())
  .then(receipt => showConfirmation(receipt));
```

### 4. Write the backend (optional)

Each endpoint is one TOML file at `api/<verb>.toml`. The verb is the URL segment after `/api/`.

#### Kind: `static_json`

Returns a constant JSON value. Useful for a product list, a feature flag, anything the deploy can hard-code.

```toml
# api/list_items.toml
kind = "static_json"

[[response]]
id    = "mug-12oz"
name  = "Ceramic mug"
price = 18

[[response]]
id    = "bowl-3pc"
name  = "Bowl set (3-piece)"
price = 42
```

Browser GET on `/api/list_items` receives the array as JSON.

#### Kind: `echo`

Returns the request body, merged with a static `extra` object that you control. Useful for stand-in form submissions where the demo just needs to confirm "we got the data".

```toml
# api/checkout.toml
kind = "echo"

[extra]
order_id = "ord-demo-001"
status   = "confirmed"
eta      = "2026-05-10"
```

Browser POST `{"item_id": "mug-12oz", "qty": 2}` to `/api/checkout` receives:

```json
{
  "item_id": "mug-12oz",
  "qty": 2,
  "order_id": "ord-demo-001",
  "status": "confirmed",
  "eta": "2026-05-10"
}
```

The merge rule: if both body and `extra` are JSON objects, fields combine (extra wins on collision). Otherwise the body lands under `input` and `extra`'s keys are sibling fields.

### 5. Deploy

Run from any cwd (the absolute folder path is what matters):

```bash
easynet pages create <project_id> --folder ~/.easynet/web-apps/<project_id>
```

You'll see something like:

```
Published.
  project_ura:  easynet:///r/easynet.run/resource/<user>.<project_id>/
  url_root:     http://<project_id>.<user>.pages.localhost:8787/
```

Open `url_root` (or `url_root + path` for a specific page) in a browser. Or:

```bash
open "$(easynet pages url <project_id>)/index.html"
```

### 6. Iterate

Edit any file under `~/.easynet/web-apps/<project_id>/` in place — the Hub reads from disk on every request. Refresh the browser. No "rebuild" step, no "redeploy" step. The fd to the published folder is held open by the daemon, so `git pull` / `npm run build` over the same folder is fine; what changes on disk is what the next request sees.

### 7. Inspect / unpublish

```bash
easynet pages list                              # all published projects
easynet pages show <project_id>                 # detail (folder, visibility, URL, size cap)
easynet pages url <project_id>                  # just the URL (scriptable)
easynet pages delete <project_id> --force       # unpublish (drops the fd, removes ability)
```

`delete` requires `--force` because it is destructive — the daemon keeps no journal of past publishes, and a re-publish needs the source folder still on disk.

## Worked example: a 4-file mini shop

```bash
mkdir -p ~/.easynet/web-apps/shop/api ~/.easynet/web-apps/shop/assets
```

`~/.easynet/web-apps/shop/index.html`:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="style.css">
  <title>EasyNet Shop</title>
</head>
<body>
  <header><h1>Ceramic Shop</h1></header>
  <main id="items"></main>
  <aside id="cart"><h3>Cart</h3><ul id="cart-items"></ul>
    <button id="checkout">Checkout</button>
    <pre id="receipt"></pre>
  </aside>
  <script src="app.js"></script>
</body>
</html>
```

`~/.easynet/web-apps/shop/style.css`:

```css
body { font-family: ui-sans-serif, system-ui, sans-serif;
       max-width: 960px; margin: 2rem auto; padding: 0 1rem; }
header h1 { color: #cc0033; }
main { display: grid; grid-template-columns: repeat(2, 1fr); gap: 1rem; }
.card { padding: 1rem; border: 1px solid #ddd; border-radius: 6px; }
aside { margin-top: 2rem; padding: 1rem; background: #f6f6f6; border-radius: 6px; }
button { padding: 0.5rem 1rem; font-size: 1rem; }
```

`~/.easynet/web-apps/shop/app.js`:

```js
const cart = [];

fetch('/api/list_items')
  .then(r => r.json())
  .then(items => {
    document.getElementById('items').innerHTML = items.map(i => `
      <div class="card">
        <h3>${i.name}</h3>
        <p>$${i.price}</p>
        <button data-id="${i.id}" data-name="${i.name}" data-price="${i.price}">Add to cart</button>
      </div>`).join('');
  });

document.addEventListener('click', e => {
  if (e.target.matches('button[data-id]')) {
    cart.push({ id: e.target.dataset.id, name: e.target.dataset.name, price: +e.target.dataset.price });
    document.getElementById('cart-items').innerHTML =
      cart.map(c => `<li>${c.name} — $${c.price}</li>`).join('');
  }
});

document.getElementById('checkout').addEventListener('click', () => {
  fetch('/api/checkout', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ items: cart, total: cart.reduce((s, c) => s + c.price, 0) }),
  })
    .then(r => r.json())
    .then(receipt => {
      document.getElementById('receipt').textContent = JSON.stringify(receipt, null, 2);
    });
});
```

`~/.easynet/web-apps/shop/api/list_items.toml`:

```toml
kind = "static_json"

[[response]]
id    = "mug-12oz"
name  = "Ceramic mug (12oz)"
price = 18

[[response]]
id    = "bowl-3pc"
name  = "Bowl set (3-piece)"
price = 42

[[response]]
id    = "vase-tall"
name  = "Tall vase"
price = 65
```

`~/.easynet/web-apps/shop/api/checkout.toml`:

```toml
kind = "echo"

[extra]
order_id = "ord-demo-001"
status   = "confirmed"
eta      = "2026-05-10"
```

Deploy:

```bash
easynet pages create shop --folder ~/.easynet/web-apps/shop
open "$(easynet pages url shop)/index.html"
```

You should see a two-column shop layout. Add to cart, hit Checkout, the receipt JSON appears.

## Common errors and what they mean

| Error                                             | What to do                                                                  |
|---------------------------------------------------|-----------------------------------------------------------------------------|
| `project already published`                        | Either you mean to update — edit files in place, no redeploy needed — or `easynet pages delete <id> --force` then re-create. |
| `folder does not exist`                            | Create the folder first; the deploy needs an existing absolute path.        |
| `dotfile path component refused`                   | A request hit `/.git/...` or `/.env`. Move the file out of the published folder; the sandbox refuses dotfiles by design. |
| `path escapes published root` / `traverses symlink` | A symlink points outside the folder. The kernel refuses; do not "fix" it. |
| `visibility 'private' is not yet supported`        | v0 ships PUBLIC only. PRIVATE/SCOPED arrives in a follow-up release.        |
| `api error: file not found`                        | The `/api/<verb>` was called with a verb whose `api/<verb>.toml` does not exist. Either create the manifest or stop calling that path. |
| `api error: malformed api manifest`                | The TOML is invalid. `cargo install taplo-cli` and run `taplo check` on it. |

## Boundaries

This skill ships static bytes + the two declarative API kinds. It does not:

- Run a Node.js / Python / Go / any runtime per request. (For that: write a real ability and publish it through `easynet ability`.)
- Render a server-side template. (Pre-render static or do it client-side.)
- Provide auth, sessions, or per-user state. (Visibility=PRIVATE, when it lands, gives capability-URL semantics; multi-user state lives in real abilities.)
- Handle file uploads. (Multipart bodies are not parsed; v0 receives JSON only.)

If the user needs any of the above and your judgment is "this isn't a small demo any more", say so plainly and offer to write a real ability (`easynet ability` family) instead of stretching this skill.

## What you will have produced

After one round of this skill the user has:

1. A folder under `~/.easynet/web-apps/<project_id>/` with their site.
2. A live URL `http://<project_id>.<user>.pages.localhost:<port>/` they can open in a browser.
3. A REAL endpoint at `/api/<verb>` for each TOML manifest they wrote.
4. The ability to edit any file in place; the daemon picks it up on the next request.

That's the deploy. That's the site. They can demo it, share the URL with anyone on the same machine, or close the loop and iterate.
