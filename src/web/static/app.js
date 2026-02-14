// Scope Web UI — Client Application
// ===================================

const API = '';  // Same origin

// ===== Navigation =====
document.querySelectorAll('nav button').forEach(function(btn) {
  btn.addEventListener('click', function() {
    document.querySelectorAll('nav button').forEach(function(b) { b.classList.remove('active'); });
    document.querySelectorAll('.panel').forEach(function(p) { p.classList.remove('active'); });
    btn.classList.add('active');
    var panel = document.getElementById('panel-' + btn.dataset.panel);
    if (panel) panel.classList.add('active');
  });
});

// ===== Safe DOM Helpers =====
function clearElement(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
}

function showLoading(el) {
  clearElement(el);
  var div = document.createElement('div');
  div.className = 'loading';
  div.textContent = 'Fetching data...';
  el.appendChild(div);
}

function showResults(el, data) {
  clearElement(el);
  var json = typeof data === 'string' ? data : JSON.stringify(data, null, 2);

  var container = document.createElement('div');
  container.className = 'results';

  var header = document.createElement('div');
  header.className = 'results-header';

  var label = document.createElement('span');
  label.textContent = 'JSON Response';
  header.appendChild(label);

  var copyBtn = document.createElement('button');
  copyBtn.className = 'btn btn-secondary';
  copyBtn.style.cssText = 'padding:4px 10px;font-size:11px;';
  copyBtn.textContent = 'Copy';
  copyBtn.addEventListener('click', function() {
    navigator.clipboard.writeText(json);
    copyBtn.textContent = 'Copied!';
    setTimeout(function() { copyBtn.textContent = 'Copy'; }, 1500);
  });
  header.appendChild(copyBtn);

  var pre = document.createElement('pre');
  pre.textContent = json;

  container.appendChild(header);
  container.appendChild(pre);
  el.appendChild(container);
}

function showError(el, msg) {
  clearElement(el);
  var container = document.createElement('div');
  container.className = 'results';
  var pre = document.createElement('pre');
  pre.className = 'error';
  pre.textContent = msg;
  container.appendChild(pre);
  el.appendChild(container);
}

async function apiPost(endpoint, body, resultEl) {
  showLoading(resultEl);
  try {
    var res = await fetch(API + endpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    var data = await res.json();
    if (data.error) {
      showError(resultEl, 'Error: ' + data.error);
    } else {
      showResults(resultEl, data);
    }
  } catch (e) {
    showError(resultEl, 'Request failed: ' + e.message);
  }
}

async function apiGet(endpoint, resultEl) {
  showLoading(resultEl);
  try {
    var res = await fetch(API + endpoint);
    var data = await res.json();
    if (data.error) {
      showError(resultEl, 'Error: ' + data.error);
    } else {
      showResults(resultEl, data);
    }
  } catch (e) {
    showError(resultEl, 'Request failed: ' + e.message);
  }
}

// ===== Command Handlers =====

function runInsights() {
  var target = document.getElementById('insights-target').value.trim();
  if (!target) return;
  var chain = document.getElementById('insights-chain').value || undefined;
  apiPost('/api/insights', { target: target, chain: chain }, document.getElementById('insights-results'));
}

function runAddress() {
  var address = document.getElementById('addr-address').value.trim();
  if (!address) return;
  apiPost('/api/address', {
    address: address,
    chain: document.getElementById('addr-chain').value,
    include_tokens: document.getElementById('addr-tokens').checked,
    include_txs: document.getElementById('addr-txs').checked,
    dossier: document.getElementById('addr-dossier').checked,
  }, document.getElementById('address-results'));
}

function runTx() {
  var hash = document.getElementById('tx-hash').value.trim();
  if (!hash) return;
  apiPost('/api/tx', {
    hash: hash,
    chain: document.getElementById('tx-chain').value,
    decode: document.getElementById('tx-decode').checked,
    trace: document.getElementById('tx-trace').checked,
  }, document.getElementById('tx-results'));
}

function runCrawl() {
  var token = document.getElementById('crawl-token').value.trim();
  if (!token) return;
  apiPost('/api/crawl', {
    token: token,
    chain: document.getElementById('crawl-chain').value,
    period: document.getElementById('crawl-period').value,
  }, document.getElementById('crawl-results'));
}

function runDiscover() {
  var source = document.getElementById('disc-source').value;
  var chain = document.getElementById('disc-chain').value;
  var limit = document.getElementById('disc-limit').value;
  var url = '/api/discover?source=' + source + '&limit=' + limit;
  if (chain) url += '&chain=' + chain;
  apiGet(url, document.getElementById('discover-results'));
}

function runTokenHealth() {
  var token = document.getElementById('th-token').value.trim();
  if (!token) return;
  apiPost('/api/token-health', {
    token: token,
    chain: document.getElementById('th-chain').value,
    with_market: document.getElementById('th-market').checked,
    market_venue: document.getElementById('th-venue').value,
  }, document.getElementById('th-results'));
}

function runMarket() {
  var pair = document.getElementById('mkt-pair').value.trim();
  if (!pair) return;
  apiPost('/api/market/summary', {
    pair: pair,
    market_venue: document.getElementById('mkt-venue').value,
    peg: parseFloat(document.getElementById('mkt-peg').value) || 1.0,
  }, document.getElementById('market-results'));
}

function runCompliance() {
  var address = document.getElementById('comp-address').value.trim();
  if (!address) return;
  apiPost('/api/compliance/risk', {
    address: address,
    chain: document.getElementById('comp-chain').value,
    detailed: document.getElementById('comp-detailed').checked,
  }, document.getElementById('compliance-results'));
}

function runExport() {
  var address = document.getElementById('exp-address').value.trim();
  if (!address) return;
  apiPost('/api/export', {
    address: address,
    chain: document.getElementById('exp-chain').value,
    format: 'json',
  }, document.getElementById('export-results'));
}

// ===== Address Book =====
function loadAddressBook() {
  apiGet('/api/address-book/list', document.getElementById('ab-results'));
}

function showAddAddress() {
  var el = document.getElementById('ab-add');
  el.style.display = el.style.display === 'none' ? 'block' : 'none';
}

function addAddressBookEntry() {
  var address = document.getElementById('ab-address').value.trim();
  if (!address) return;
  apiPost('/api/address-book/add', {
    address: address,
    chain: document.getElementById('ab-chain').value,
    label: document.getElementById('ab-label').value || undefined,
  }, document.getElementById('ab-results'));
}

// ===== Venue Loading =====
var venuesCache = null;

async function loadVenues() {
  try {
    var res = await fetch(API + '/api/venues');
    var data = await res.json();
    if (data.venues) {
      venuesCache = data.venues;
      populateVenueSelects(data.venues);
    }
  } catch (e) {
    // Silently fall back to hardcoded options
  }
}

function populateVenueSelects(venues) {
  document.querySelectorAll('.venue-select').forEach(function(sel) {
    // Preserve the first option if it's a "None" placeholder
    var hasNone = sel.options.length > 0 && sel.options[0].value === '';
    var currentValue = sel.value;

    while (sel.options.length > (hasNone ? 1 : 0)) {
      sel.remove(hasNone ? 1 : 0);
    }

    // Add CEX venues from registry
    venues.forEach(function(v) {
      var opt = document.createElement('option');
      opt.value = v.id;
      opt.textContent = v.name;
      sel.appendChild(opt);
    });

    // Add DEX venues (not from registry)
    var dexVenues = [
      { id: 'eth', name: 'Ethereum DEX' },
      { id: 'solana', name: 'Solana DEX' },
    ];
    dexVenues.forEach(function(d) {
      var opt = document.createElement('option');
      opt.value = d.id;
      opt.textContent = d.name;
      sel.appendChild(opt);
    });

    // Restore selection
    if (currentValue) sel.value = currentValue;
  });
}

// ===== Exchange Snapshot =====
function runExchange() {
  var venue = document.getElementById('ex-venue').value;
  var pair = document.getElementById('ex-pair').value.trim() || 'BTC';
  var resultEl = document.getElementById('exchange-results');
  showLoading(resultEl);

  fetch(API + '/api/exchange/snapshot', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ venue: venue, pair: pair }),
  }).then(function(res) { return res.json(); })
    .then(function(data) {
      if (data.error) {
        showError(resultEl, 'Error: ' + data.error);
        return;
      }
      renderExchangeSnapshot(resultEl, data);
    })
    .catch(function(e) {
      showError(resultEl, 'Request failed: ' + e.message);
    });
}

function renderExchangeSnapshot(el, data) {
  clearElement(el);

  var grid = document.createElement('div');
  grid.className = 'exchange-grid';

  // Left column: Ticker + Order Book
  var leftCol = document.createElement('div');
  leftCol.className = 'exchange-col';

  // Ticker
  if (data.ticker) {
    var tickerH3 = document.createElement('h3');
    tickerH3.textContent = 'Ticker — ' + (data.pair || '');
    leftCol.appendChild(tickerH3);
    renderTicker(leftCol, data.ticker);
  }

  // Order Book
  if (data.order_book) {
    var obH3 = document.createElement('h3');
    obH3.style.marginTop = '12px';
    obH3.textContent = 'Order Book';
    leftCol.appendChild(obH3);
    renderOrderBook(leftCol, data.order_book);
  }

  grid.appendChild(leftCol);

  // Right column: Trade History
  var rightCol = document.createElement('div');
  rightCol.className = 'exchange-col';
  var trH3 = document.createElement('h3');
  trH3.textContent = 'Recent Trades';
  rightCol.appendChild(trH3);

  if (data.recent_trades && data.recent_trades.length > 0) {
    renderTradeHistory(rightCol, data.recent_trades);
  } else {
    var empty = document.createElement('div');
    empty.style.color = 'var(--text-muted)';
    empty.style.fontFamily = 'var(--font-mono)';
    empty.style.fontSize = '12px';
    empty.textContent = 'No trade data available for this venue.';
    rightCol.appendChild(empty);
  }

  grid.appendChild(rightCol);
  el.appendChild(grid);
}

function renderTicker(parent, ticker) {
  var fields = [
    ['Last Price', formatPrice(ticker.last_price)],
    ['24h High', formatPrice(ticker.high_24h)],
    ['24h Low', formatPrice(ticker.low_24h)],
    ['24h Volume', fmtCompact(ticker.volume_24h)],
    ['Quote Volume', fmtCompact(ticker.quote_volume_24h)],
    ['Best Bid', formatPrice(ticker.best_bid)],
    ['Best Ask', formatPrice(ticker.best_ask)],
  ];
  fields.forEach(function(f) {
    var row = document.createElement('div');
    row.className = 'ticker-row';
    var lbl = document.createElement('span');
    lbl.className = 'ticker-label';
    lbl.textContent = f[0];
    var val = document.createElement('span');
    val.className = 'ticker-value';
    val.textContent = f[1];
    row.appendChild(lbl);
    row.appendChild(val);
    parent.appendChild(row);
  });
}

function renderOrderBook(parent, ob) {
  var table = document.createElement('table');
  table.className = 'ob-table';

  // Header
  var thead = document.createElement('thead');
  var hr = document.createElement('tr');
  ['Price', 'Quantity', 'Value'].forEach(function(h) {
    var th = document.createElement('th');
    th.textContent = h;
    hr.appendChild(th);
  });
  thead.appendChild(hr);
  table.appendChild(thead);

  var tbody = document.createElement('tbody');

  // Asks (reversed so lowest ask is at bottom, closest to spread)
  var asks = (ob.asks || []).slice(0, 10).reverse();
  asks.forEach(function(level) {
    var tr = document.createElement('tr');
    tr.className = 'ob-ask';
    addOBCell(tr, formatPrice(level.price));
    addOBCell(tr, fmtQty(level.quantity));
    addOBCell(tr, fmtCompact(level.value));
    tbody.appendChild(tr);
  });

  // Spread row
  if (ob.spread !== null && ob.spread !== undefined) {
    var sr = document.createElement('tr');
    sr.className = 'ob-spread';
    var sd = document.createElement('td');
    sd.colSpan = 3;
    sd.textContent = 'Spread: ' + formatPrice(ob.spread) +
      (ob.mid_price ? ' | Mid: ' + formatPrice(ob.mid_price) : '');
    sr.appendChild(sd);
    tbody.appendChild(sr);
  }

  // Bids
  var bids = (ob.bids || []).slice(0, 10);
  bids.forEach(function(level) {
    var tr = document.createElement('tr');
    tr.className = 'ob-bid';
    addOBCell(tr, formatPrice(level.price));
    addOBCell(tr, fmtQty(level.quantity));
    addOBCell(tr, fmtCompact(level.value));
    tbody.appendChild(tr);
  });

  table.appendChild(tbody);
  parent.appendChild(table);
}

function addOBCell(row, text) {
  var td = document.createElement('td');
  td.textContent = text;
  row.appendChild(td);
}

function renderTradeHistory(parent, trades) {
  // Header row
  var hdr = document.createElement('div');
  hdr.className = 'trade-row';
  hdr.style.fontWeight = '600';
  hdr.style.color = 'var(--text-muted)';
  hdr.style.fontSize = '10px';
  hdr.style.textTransform = 'uppercase';
  ['Side', 'Price', 'Qty', 'Time'].forEach(function(h) {
    var sp = document.createElement('span');
    sp.className = h === 'Side' ? 'trade-side' :
                   h === 'Price' ? 'trade-price' :
                   h === 'Qty' ? 'trade-qty' : 'trade-time';
    sp.textContent = h;
    hdr.appendChild(sp);
  });
  parent.appendChild(hdr);

  trades.slice(0, 50).forEach(function(t) {
    var row = document.createElement('div');
    row.className = 'trade-row ' + (t.side === 'buy' ? 'trade-buy' : 'trade-sell');

    var side = document.createElement('span');
    side.className = 'trade-side';
    side.textContent = t.side === 'buy' ? 'B' : 'S';
    row.appendChild(side);

    var price = document.createElement('span');
    price.className = 'trade-price';
    price.textContent = formatPrice(t.price);
    row.appendChild(price);

    var qty = document.createElement('span');
    qty.className = 'trade-qty';
    qty.textContent = fmtQty(t.quantity);
    row.appendChild(qty);

    var time = document.createElement('span');
    time.className = 'trade-time';
    time.textContent = formatTradeTime(t.timestamp_ms);
    row.appendChild(time);

    parent.appendChild(row);
  });
}

// ===== Formatting Helpers =====
function formatPrice(n) {
  if (n === null || n === undefined) return '-';
  if (n >= 1000) return n.toLocaleString(undefined, { maximumFractionDigits: 2 });
  if (n >= 1) return n.toFixed(4);
  if (n >= 0.01) return n.toFixed(6);
  return n.toFixed(8);
}

function fmtCompact(n) {
  if (n === null || n === undefined) return '-';
  if (n >= 1e9) return (n / 1e9).toFixed(2) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(2) + 'K';
  return n.toFixed(2);
}

function fmtQty(n) {
  if (n === null || n === undefined) return '-';
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(2) + 'K';
  if (n >= 1) return n.toFixed(4);
  return n.toFixed(6);
}

function formatTradeTime(ms) {
  if (!ms) return '-';
  var d = new Date(ms);
  return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

// ===== Monitor (WebSocket) =====
var monitorWs = null;
var priceHistory = [];

function toggleMonitor() {
  if (monitorWs) {
    monitorWs.close();
    monitorWs = null;
    document.getElementById('mon-btn').textContent = 'Start';
    document.getElementById('mon-container').style.display = 'none';
    document.getElementById('mon-exchange').style.display = 'none';
    return;
  }

  var token = document.getElementById('mon-token').value.trim();
  if (!token) return;

  var chain = document.getElementById('mon-chain').value;
  var refresh = document.getElementById('mon-refresh').value || 5;
  var venue = document.getElementById('mon-venue').value || '';
  var pair = document.getElementById('mon-pair').value.trim() || '';
  var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  var url = proto + '//' + location.host + '/ws/monitor?token=' + encodeURIComponent(token) + '&chain=' + chain + '&refresh=' + refresh;
  if (venue) url += '&venue=' + encodeURIComponent(venue);
  if (pair) url += '&pair=' + encodeURIComponent(pair);

  priceHistory = [];
  monitorWs = new WebSocket(url);
  document.getElementById('mon-btn').textContent = 'Stop';
  document.getElementById('mon-container').style.display = 'grid';

  // Show exchange panel if venue is set
  if (venue) {
    document.getElementById('mon-exchange').style.display = 'grid';
  }

  monitorWs.onmessage = function(event) {
    var data = JSON.parse(event.data);
    if (data.type === 'update') {
      updateMonitorDisplay(data);
    } else if (data.type === 'error') {
      var statsEl = document.getElementById('mon-stats');
      clearElement(statsEl);
      var card = document.createElement('div');
      card.className = 'stat-card';
      var val = document.createElement('div');
      val.className = 'stat-value error';
      val.textContent = data.message;
      card.appendChild(val);
      statsEl.appendChild(card);
      // Still render exchange data if present
      if (data.exchange_order_book || data.exchange_trades) {
        updateMonitorExchange(data);
      }
    }
  };

  monitorWs.onclose = function() {
    monitorWs = null;
    document.getElementById('mon-btn').textContent = 'Start';
  };
}

function updateMonitorDisplay(data) {
  // Update price history for chart
  priceHistory.push(data.price_usd);
  if (priceHistory.length > 60) priceHistory.shift();

  // Draw simple price chart on canvas
  var canvas = document.getElementById('price-canvas');
  var ctx = canvas.getContext('2d');
  var w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);

  if (priceHistory.length > 1) {
    var min = Math.min.apply(null, priceHistory) * 0.999;
    var max = Math.max.apply(null, priceHistory) * 1.001;
    var range = max - min || 1;

    // Grid lines
    ctx.strokeStyle = '#30363d';
    ctx.lineWidth = 0.5;
    for (var i = 0; i < 5; i++) {
      var gy = (h / 5) * i;
      ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(w, gy); ctx.stroke();
    }

    // Price line
    ctx.strokeStyle = '#58a6ff';
    ctx.lineWidth = 2;
    ctx.beginPath();
    priceHistory.forEach(function(p, idx) {
      var x = (idx / (priceHistory.length - 1)) * w;
      var y = h - ((p - min) / range) * h;
      if (idx === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    });
    ctx.stroke();

    // Price label
    ctx.fillStyle = '#f0f6fc';
    ctx.font = '14px monospace';
    ctx.fillText('$' + data.price_usd.toFixed(6), 8, 20);
  }

  // Stats cards
  var statsEl = document.getElementById('mon-stats');
  clearElement(statsEl);

  var change24 = data.price_change_24h || 0;
  var changeClass = change24 >= 0 ? 'positive' : 'negative';
  var changeSign = change24 >= 0 ? '+' : '';

  function addStat(label, value, extra) {
    var card = document.createElement('div');
    card.className = 'stat-card';

    var lbl = document.createElement('div');
    lbl.className = 'stat-label';
    lbl.textContent = label;
    card.appendChild(lbl);

    var val = document.createElement('div');
    val.className = 'stat-value';
    if (typeof value === 'object' && value !== null) {
      val.appendChild(value);
    } else {
      val.textContent = value;
    }
    card.appendChild(val);

    if (extra) {
      var ext = document.createElement('div');
      ext.className = 'stat-change ' + extra.cls;
      ext.textContent = extra.text;
      card.appendChild(ext);
    }

    statsEl.appendChild(card);
  }

  addStat((data.token ? data.token.symbol : '') + ' Price',
    '$' + (data.price_usd ? data.price_usd.toFixed(6) : '-'),
    { cls: changeClass, text: changeSign + change24.toFixed(2) + '% (24h)' });
  addStat('Volume (24h)', fmtCompact(data.volume_24h));
  addStat('Liquidity', fmtCompact(data.liquidity_usd));
  addStat('Market Cap', fmtCompact(data.market_cap));

  // Buys/Sells as a compound element
  var bsSpan = document.createElement('span');
  bsSpan.style.fontSize = '16px';
  var buySpan = document.createElement('span');
  buySpan.style.color = 'var(--green)';
  buySpan.textContent = String(data.buys_24h || 0);
  var sellSpan = document.createElement('span');
  sellSpan.style.color = 'var(--red)';
  sellSpan.textContent = String(data.sells_24h || 0);
  bsSpan.appendChild(buySpan);
  bsSpan.appendChild(document.createTextNode(' / '));
  bsSpan.appendChild(sellSpan);
  addStat('Buys / Sells (24h)', bsSpan);

  addStat('Last Update', data.timestamp || '-');

  // Exchange ticker as stat card (if available)
  if (data.exchange_ticker) {
    var t = data.exchange_ticker;
    addStat('CEX Price', formatPrice(t.last_price));
    if (t.volume_24h) addStat('CEX Volume', fmtCompact(t.volume_24h));
  }

  // Render exchange data (order book / trades)
  if (data.exchange_order_book || data.exchange_trades) {
    updateMonitorExchange(data);
  }
}

function updateMonitorExchange(data) {
  var panel = document.getElementById('mon-exchange');
  if (!panel) return;
  panel.style.display = 'grid';

  // Order book
  var obEl = document.getElementById('mon-orderbook');
  clearElement(obEl);
  if (data.exchange_order_book) {
    var h4 = document.createElement('h4');
    h4.textContent = 'Order Book';
    obEl.appendChild(h4);
    renderOrderBook(obEl, data.exchange_order_book);
  }

  // Trades
  var trEl = document.getElementById('mon-trades');
  clearElement(trEl);
  if (data.exchange_trades && data.exchange_trades.length > 0) {
    var h4t = document.createElement('h4');
    h4t.textContent = 'Recent Trades';
    trEl.appendChild(h4t);
    renderTradeHistory(trEl, data.exchange_trades);
  }
}

// ===== Setup =====
async function loadConfigStatus() {
  try {
    var res = await fetch(API + '/api/config/status');
    var data = await res.json();

    document.getElementById('version').textContent = 'v' + (data.version || '?');

    var statusEl = document.getElementById('setup-status');
    clearElement(statusEl);

    // Config file info
    var fileInfo = document.createElement('div');
    fileInfo.style.marginBottom = '12px';

    var strong = document.createElement('strong');
    strong.textContent = 'Config file: ';
    fileInfo.appendChild(strong);

    var code = document.createElement('code');
    code.textContent = data.config_path || 'unknown';
    fileInfo.appendChild(code);
    fileInfo.appendChild(document.createTextNode(' '));

    var badge = document.createElement('span');
    badge.className = data.config_exists ? 'badge badge-set' : 'badge badge-unset';
    badge.textContent = data.config_exists ? 'exists' : 'not found';
    fileInfo.appendChild(badge);

    statusEl.appendChild(fileInfo);

    // API Keys section
    var keys = data.api_keys || {};
    var keysHeader = document.createElement('h3');
    keysHeader.style.cssText = 'color:var(--text-bright);margin-bottom:8px;';
    keysHeader.textContent = 'API Keys';
    statusEl.appendChild(keysHeader);

    var keysGrid = document.createElement('div');
    keysGrid.className = 'setup-grid';
    Object.entries(keys).forEach(function(entry) {
      var card = document.createElement('div');
      card.className = 'setup-card';
      var h3 = document.createElement('h3');
      h3.textContent = entry[0];
      card.appendChild(h3);
      var b = document.createElement('span');
      b.className = entry[1] ? 'badge badge-set' : 'badge badge-unset';
      b.textContent = entry[1] ? 'Configured' : 'Not set';
      card.appendChild(b);
      keysGrid.appendChild(card);
    });
    statusEl.appendChild(keysGrid);

    // RPC Endpoints section
    var rpcs = data.rpc_endpoints || {};
    var rpcsHeader = document.createElement('h3');
    rpcsHeader.style.cssText = 'color:var(--text-bright);margin:12px 0 8px;';
    rpcsHeader.textContent = 'RPC Endpoints';
    statusEl.appendChild(rpcsHeader);

    var rpcsGrid = document.createElement('div');
    rpcsGrid.className = 'setup-grid';
    Object.entries(rpcs).forEach(function(entry) {
      var card = document.createElement('div');
      card.className = 'setup-card';
      var h3 = document.createElement('h3');
      h3.textContent = entry[0];
      card.appendChild(h3);
      var b = document.createElement('span');
      b.className = entry[1] ? 'badge badge-set' : 'badge badge-unset';
      b.textContent = entry[1] ? 'Configured' : 'Not set';
      card.appendChild(b);
      rpcsGrid.appendChild(card);
    });
    statusEl.appendChild(rpcsGrid);
  } catch (e) {
    var statusEl2 = document.getElementById('setup-status');
    clearElement(statusEl2);
    var errDiv = document.createElement('div');
    errDiv.className = 'error';
    errDiv.textContent = 'Failed to load config status';
    statusEl2.appendChild(errDiv);
  }
}

async function saveConfig() {
  var api_keys = {};
  var fields = ['etherscan', 'polygonscan', 'bscscan', 'solscan', 'tronscan'];
  fields.forEach(function(f) {
    var val = document.getElementById('key-' + f).value.trim();
    if (val) api_keys[f] = val;
  });

  try {
    var res = await fetch(API + '/api/config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ api_keys: api_keys, rpc_endpoints: {} }),
    });
    var data = await res.json();
    if (data.error) {
      alert('Error: ' + data.error);
    } else {
      alert('Configuration saved to ' + data.path);
      loadConfigStatus();
    }
  } catch (e) {
    alert('Failed to save: ' + e.message);
  }
}

// ===== Enter key support =====
document.querySelectorAll('input[type="text"]').forEach(function(input) {
  input.addEventListener('keydown', function(e) {
    if (e.key === 'Enter') {
      var panel = input.closest('.panel');
      if (panel) {
        var btn = panel.querySelector('.btn');
        if (btn) btn.click();
      }
    }
  });
});

// ===== Init =====
loadConfigStatus();
loadAddressBook();
loadVenues();
