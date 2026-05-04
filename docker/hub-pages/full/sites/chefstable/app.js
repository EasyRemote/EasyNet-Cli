const MONTHS = ['JAN','FEB','MAR','APR','MAY','JUN','JUL','AUG','SEP','OCT','NOV','DEC'];

let MENUS = [];

fetch('/api/upcoming_menus')
  .then(r => r.json())
  .then(menus => {
    MENUS = menus;
    renderMenus(menus);
    populateMenuSelect(menus);
  })
  .catch(err => {
    document.getElementById('menus').innerHTML =
      `<div class="loading">Calendar temporarily unavailable. Please write concierge@chefstable.nyc.</div>`;
    console.error(err);
  });

function renderMenus(menus) {
  const html = menus.map(m => {
    const d = new Date(m.date + 'T00:00:00');
    const day = d.getDate();
    const month = MONTHS[d.getMonth()];
    const soldOut = m.seats_remaining === 0;
    const seatsClass = soldOut ? 'sold-out' : '';
    const seatsText = soldOut
      ? '<span class="seats-num">Sold out</span>'
      : `<span class="seats-num">${m.seats_remaining}</span> <span class="seats-lbl">of ${m.seats_total} remain</span>`;

    return `
      <article class="menu-card">
        <div class="menu-date">
          <span class="day">${m.day}</span>
          <span class="num-date">${day}</span>
          <span class="month">${month} 2026</span>
        </div>
        <div class="menu-body">
          <h3>${escapeHtml(m.title)}</h3>
          <div class="chef">${escapeHtml(m.chef)}</div>
          <p class="theme">${escapeHtml(m.theme)}</p>
          <ul class="courses">
            ${m.courses.map(c => `<li>${escapeHtml(c)}</li>`).join('')}
          </ul>
          <div class="pairing">${escapeHtml(m.pairing)}</div>
        </div>
        <div class="menu-meta ${seatsClass}">
          <div class="price">$${m.price}</div>
          <span class="price-lbl">per guest</span>
          <div class="seats">
            ${seatsText}
          </div>
        </div>
      </article>
    `;
  }).join('');

  document.getElementById('menus').innerHTML = html;
}

function populateMenuSelect(menus) {
  const select = document.getElementById('menu-select');
  menus.forEach(m => {
    const d = new Date(m.date + 'T00:00:00');
    const opt = document.createElement('option');
    opt.value = m.id;
    const label = `${m.day}, ${MONTHS[d.getMonth()]} ${d.getDate()} — ${m.title} ($${m.price})`;
    opt.textContent = m.seats_remaining === 0 ? `${label} — SOLD OUT` : label;
    if (m.seats_remaining === 0) opt.disabled = true;
    select.appendChild(opt);
  });
}

document.getElementById('reservation-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const form = e.currentTarget;
  const data = Object.fromEntries(new FormData(form).entries());
  data.party_size = parseInt(data.party_size, 10);
  const menu = MENUS.find(m => m.id === data.menu_id);
  if (menu) {
    data.evening = `${menu.day}, ${menu.date}`;
    data.menu_title = menu.title;
    data.estimated_total = menu.price * data.party_size;
  }

  const btn = form.querySelector('button[type="submit"]');
  btn.disabled = true;
  btn.querySelector('.cta-label').textContent = 'Sending to the kitchen…';

  try {
    const res = await fetch('/api/reserve', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    });
    const receipt = await res.json();
    showConfirmation(receipt);
    form.reset();
  } catch (err) {
    alert('Something went wrong. Please try again or write concierge@chefstable.nyc.');
    console.error(err);
  } finally {
    btn.disabled = false;
    btn.querySelector('.cta-label').textContent = 'Request the table';
  }
});

function showConfirmation(r) {
  const partyText = r.party_size === 1 ? 'one seat' : `${r.party_size} seats`;
  const html = `
    <p class="ok-eyebrow">Reservation Confirmed</p>
    <h3>The chef is expecting you.</h3>
    <p class="greeting">${escapeHtml(r.name || 'Welcome')}, your seat at the long table is held.</p>
    <code class="booking-id">${escapeHtml(r.booking_id)}</code>
    <dl>
      <div><dt>Evening</dt><dd>${escapeHtml(r.evening || '—')}</dd></div>
      <div><dt>Menu</dt><dd>${escapeHtml(r.menu_title || '—')}</dd></div>
      <div><dt>Party</dt><dd>${partyText}</dd></div>
      <div><dt>Estimated total</dt><dd>$${r.estimated_total ?? '—'}</dd></div>
      <div><dt>Arrival window</dt><dd>${escapeHtml(r.arrival_window)}</dd></div>
      <div><dt>Dress</dt><dd>${escapeHtml(r.dress_code)}</dd></div>
    </dl>
    <p class="footnote">${escapeHtml(r.location_hint)}.<br>A confirmation has been emailed to <strong>${escapeHtml(r.email || '')}</strong>. ${escapeHtml(r.cancellation_policy)}.</p>
  `;
  const el = document.getElementById('confirmation');
  el.innerHTML = html;
  el.classList.remove('hidden');
  el.scrollIntoView({ behavior: 'smooth', block: 'center' });
}

function escapeHtml(s) {
  if (s == null) return '';
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
