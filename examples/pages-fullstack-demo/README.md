# EasyNet Pages — full-stack demo

A ready-to-publish folder with a **static frontend** and a **real backend**,
both served by your EasyNet daemon through the Hub. No build step, no nginx,
no separate API server, no LLM call — just `easynet pages create`.

Unlike `examples/public-routes-e2e/d2-agent-fullstack.sh` (which dispatches an agent to *write* an
app), this folder is checked in and publishable directly.

## Layout

```
pages-fullstack-demo/
├── index.html        # frontend markup
├── style.css         # frontend styles
├── app.js            # frontend logic — fetches the backend below
├── api/
│   ├── products.toml # GET  api/products  (kind = "static_json")
│   └── feedback.toml # POST api/feedback  (kind = "echo")
├── publish.sh        # one-command publish helper
└── README.md
```

## How the front and back connect

EasyNet Pages serves two route families under a published project root
(`https://<realm>/web/<user>/<project>/`):

| Route                | Served by                              | Source file        |
| -------------------- | -------------------------------------- | ------------------ |
| `/` and static files | `<user>.<project>.page.fetch`          | `index.html`, etc. |
| `/api/<verb>`        | `<user>.<project>.api.<verb>`          | `api/<verb>.toml`  |

`app.js` calls its own backend with **relative** URLs (`fetch("api/products")`,
`fetch("api/feedback", {method:"POST", ...})`), so the same bytes work at
whatever URL the project is published to.

Each `api/<verb>.toml` manifest is evaluated per request and returns
`{ status, body, content_type }`; the Hub turns `body` into the HTTP response
(with `Access-Control-Allow-Origin: *`). v0 manifest kinds:

- `static_json` — return a constant JSON value (used by `products.toml`).
- `echo` — return the request body merged with a static `extra` table
  (used by `feedback.toml`).
- `ability` — forward the request to a real EasyNet ability (see below).

## Publish it

```bash
# from this folder, with a running + joined daemon:
./publish.sh

# or by hand:
easynet pages create fullstack-demo --folder "$(pwd)"
easynet pages url fullstack-demo      # prints the public URL
```

Open the printed URL. The product list loads from `GET api/products`; the
feedback form posts to `POST api/feedback` and shows the backend's response.

Verify the backend directly with curl:

```bash
BASE="$(easynet pages url fullstack-demo)"
curl -s "${BASE}api/products"
curl -s -X POST "${BASE}api/feedback" \
  -H 'Content-Type: application/json' \
  -d '{"name":"silan","message":"works"}'
```

Unpublish: `easynet pages delete fullstack-demo --force`.

## Upgrading to a persistent backend

`echo` does not persist. To make `api/feedback` write to real storage, deploy
an EasyNet ability (e.g. `add_feedback`) and switch the manifest to:

```toml
# api/feedback.toml
kind = "ability"
ability_ura = "easynet:///r/<realm>/ability/<user>.<project>.api.add_feedback"
```

The HTTP request body is forwarded verbatim as the ability's args, and the
ability's result becomes the HTTP response body. See
`examples/public-routes-e2e/d2-agent-fullstack.sh` for an end-to-end `kind="ability"` example.
