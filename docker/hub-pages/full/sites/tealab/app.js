const cart = [];

const itemsEl = document.getElementById('items');
const cartListEl = document.getElementById('cart-items');
const cartTotalEl = document.getElementById('cart-total');
const checkoutBtn = document.getElementById('checkout');
const receiptEl = document.getElementById('receipt');

function fmt(n) { return '$' + n.toFixed(2); }

function renderCart() {
  if (cart.length === 0) {
    cartListEl.innerHTML = '<li class="empty" style="color:#6b7a70">Empty</li>';
    cartTotalEl.textContent = '$0';
    checkoutBtn.disabled = true;
    return;
  }
  cartListEl.innerHTML = cart.map(c =>
    `<li><span>${c.name} ×${c.qty}</span><span>${fmt(c.price * c.qty)}</span></li>`
  ).join('');
  const total = cart.reduce((s, c) => s + c.price * c.qty, 0);
  cartTotalEl.textContent = fmt(total);
  checkoutBtn.disabled = false;
}

function addToCart(item) {
  const existing = cart.find(c => c.id === item.id);
  if (existing) existing.qty += 1;
  else cart.push({ ...item, qty: 1 });
  renderCart();
}

fetch('/api/list_items')
  .then(r => {
    if (!r.ok) throw new Error('HTTP ' + r.status);
    return r.json();
  })
  .then(items => {
    if (!Array.isArray(items) || items.length === 0) {
      itemsEl.innerHTML = '<p class="loading">No teas in stock today.</p>';
      return;
    }
    itemsEl.innerHTML = items.map(i => `
      <article class="card">
        <p class="origin">${i.origin}</p>
        <h3>${i.name}</h3>
        <p class="desc">${i.description}</p>
        <div class="row">
          <span class="price">${fmt(i.price)}</span>
          <span class="grams">${i.grams}g</span>
        </div>
        <div class="row">
          <button data-id="${i.id}" data-name="${i.name}" data-price="${i.price}">Add to cart</button>
        </div>
      </article>
    `).join('');
  })
  .catch(err => {
    itemsEl.innerHTML = `<p class="loading">Could not load teas: ${err.message}</p>`;
  });

document.addEventListener('click', e => {
  const btn = e.target.closest('button[data-id]');
  if (!btn) return;
  addToCart({
    id: btn.dataset.id,
    name: btn.dataset.name,
    price: parseFloat(btn.dataset.price),
  });
});

checkoutBtn.addEventListener('click', () => {
  const total = cart.reduce((s, c) => s + c.price * c.qty, 0);
  receiptEl.textContent = 'Submitting…';
  fetch('/api/checkout', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      items: cart.map(c => ({ id: c.id, name: c.name, qty: c.qty, price: c.price })),
      total,
      currency: 'USD',
    }),
  })
    .then(r => r.json())
    .then(receipt => {
      receiptEl.textContent = JSON.stringify(receipt, null, 2);
      cart.length = 0;
      renderCart();
    })
    .catch(err => {
      receiptEl.textContent = 'Checkout failed: ' + err.message;
    });
});

renderCart();
