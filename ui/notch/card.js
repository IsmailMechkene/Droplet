// Builds the usage card for one provider from the finished view Rust sends.
// Every string is already worded and translated by Rust.

const esc = (text) => String(text ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

const FORECAST_MARK = { warn: '!', danger: '!', safe: '✓', onpace: '✓', learning: '·', stale: '·' };
const SPARK_H = 30;

function sparkline(points, width) {
  if (points.length < 2) return '';
  const xy = points.map((y, i) => [(i / (points.length - 1)) * width, 28 - (y / 100) * 26]);
  const d = 'M' + xy.map((p) => p[0].toFixed(1) + ' ' + p[1].toFixed(1)).join('L');
  const [x, y] = xy[xy.length - 1];
  return `<svg width="${width}" height="${SPARK_H}"><line class="base" x1="0" y1="29.5" x2="${width}" y2="29.5"></line>` +
    `<path class="line" d="${d}"></path><circle class="now" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="2.5"></circle></svg>`;
}

/**
 * Fills `el` with the card for `view` (a provider from the snapshot).
 * `colorFor(pct)` returns the bar colour; `suggestion` is shown in the header when it concerns this provider.
 */
export function renderCard(el, view, { strings, colorFor, suggestion }) {
  const blocks = [];
  const stale = view.card === 'stale';

  blocks.push(`<div class="card-head">
    <div class="logo"><img src="../icons/p-${view.id}.svg" alt=""></div>
    <div class="title">${esc(view.title)}</div>
    ${view.plan ? `<div class="plan">${esc(view.plan)}</div>` : ''}
  </div>`);

  if (suggestion) {
    blocks.push(`<div class="card-suggest"><span class="dot"></span><span>${esc(suggestion.a)} <b>${esc(suggestion.b)}</b></span></div>`);
  }
  if (view.message) {
    blocks.push(`<div class="card-msg"><div class="t">${esc(view.message.title)}</div><div class="b">${esc(view.message.body)}</div></div>`);
  }
  for (const row of view.rows) {
    const color = stale ? 'var(--dp-stale)' : colorFor(row.pct);
    blocks.push(`<div class="card-row" tabindex="0" role="group" aria-label="${esc(row.label)}">
      <div class="top"><div class="name">${esc(row.name)}</div><div class="reset">${esc(row.reset)}</div></div>
      <div class="bar"><div class="fill" style="width:${row.pct}%;background:${color}"></div></div>
      <div class="cap">${esc(row.caption)}</div>
    </div>`);
  }
  if (view.forecast) {
    const kind = view.forecast.kind;
    blocks.push(`<div class="card-fc ${kind}"><div class="mark">${FORECAST_MARK[kind] || '·'}</div><div class="txt">${esc(view.forecast.text)}</div></div>`);
  }
  if (view.spark.length > 1) {
    blocks.push(`<div class="card-spark">${sparkline(view.spark, 232)}
      <div class="axis"><span>${esc(strings['card.spark_from'])}</span><span>${esc(strings['card.spark_to'])}</span></div></div>`);
  }
  blocks.push(`<div class="card-foot">${stale ? '<span class="dot"></span>' : ''}<span>${esc(view.footer)}</span></div>`);

  el.classList.toggle('stale', stale);
  el.setAttribute('aria-label', view.title);
  el.innerHTML = blocks.map((html, i) => `<div class="blk" style="--i:${i}">${html}</div>`).join('');
}
