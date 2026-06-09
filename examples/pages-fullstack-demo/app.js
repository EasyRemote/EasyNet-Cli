// app.js — talks to this project's own backend.
//
// Endpoints are relative to the published root
// (https://<realm>/web/<user>/<project>/), so "api/products" resolves to
// the api/<verb> route the Hub maps to api/<verb>.toml on the daemon.
//   - GET  api/products  -> api/products.toml (kind = "static_json")
//   - POST api/feedback  -> api/feedback.toml (kind = "echo")

async function loadProducts() {
  const list = document.getElementById('products');
  const source = document.getElementById('products-source');
  try {
    const res = await fetch('api/products');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    const products = (data && data.products) || [];
    list.innerHTML = '';
    if (products.length === 0) {
      list.innerHTML = '<li class="dim">no products returned</li>';
    }
    for (const p of products) {
      const li = document.createElement('li');
      const name = document.createElement('span');
      name.textContent = p.name ?? '(unnamed)';
      const price = document.createElement('span');
      price.className = 'price';
      price.textContent = p.price ?? '';
      li.append(name, price);
      list.appendChild(li);
    }
    source.textContent = '— from GET api/products';
  } catch (err) {
    list.innerHTML = `<li class="dim">failed to load: ${err.message}</li>`;
  }
}

function wireFeedbackForm() {
  const form = document.getElementById('feedback-form');
  const out = document.getElementById('feedback-result');
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const payload = {
      name: document.getElementById('name').value,
      message: document.getElementById('message').value,
    };
    out.hidden = false;
    out.textContent = 'sending…';
    try {
      const res = await fetch('api/feedback', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      out.textContent = 'backend responded:\n' + JSON.stringify(data, null, 2);
    } catch (err) {
      out.textContent = 'failed: ' + err.message;
    }
  });
}

loadProducts();
wireFeedbackForm();
