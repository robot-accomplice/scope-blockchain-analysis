// Scope Web UI — Client Application
// ===================================

var API = '';  // Same origin

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

function el(tag, attrs, children) {
  var e = document.createElement(tag);
  if (attrs) {
    Object.keys(attrs).forEach(function(k) {
      if (k === 'className') e.className = attrs[k];
      else if (k === 'textContent') e.textContent = attrs[k];
      else if (k === 'innerHTML') e.innerHTML = attrs[k];
      else if (k.indexOf('on') === 0) e.addEventListener(k.slice(2).toLowerCase(), attrs[k]);
      else if (k === 'style' && typeof attrs[k] === 'object') Object.assign(e.style, attrs[k]);
      else e.setAttribute(k, attrs[k]);
    });
  }
  if (children) {
    (Array.isArray(children) ? children : [children]).forEach(function(c) {
      if (c == null) return;
      if (typeof c === 'string') e.appendChild(document.createTextNode(c));
      else e.appendChild(c);
    });
  }
  return e;
}

function showLoading(resultEl) {
  clearElement(resultEl);
  resultEl.appendChild(el('div', { className: 'loading' }, 'Analyzing...'));
}

function showError(resultEl, msg) {
  clearElement(resultEl);
  var container = el('div', { className: 'results' });
  container.appendChild(el('pre', { className: 'error', textContent: msg }));
  resultEl.appendChild(container);
}

// ===== Formatting Helpers =====
function fmtPrice(n) {
  if (n == null) return '-';
  if (n >= 1000) return '$' + n.toLocaleString(undefined, { maximumFractionDigits: 2 });
  if (n >= 1) return '$' + n.toFixed(4);
  if (n >= 0.01) return '$' + n.toFixed(6);
  return '$' + n.toFixed(8);
}

function fmtUsd(n) {
  if (n == null) return '-';
  if (n >= 1e9) return '$' + (n / 1e9).toFixed(2) + 'B';
  if (n >= 1e6) return '$' + (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return '$' + (n / 1e3).toFixed(2) + 'K';
  return '$' + n.toFixed(2);
}

function fmtNum(n) {
  if (n == null) return '-';
  if (typeof n === 'string') return n;
  if (n >= 1e9) return (n / 1e9).toFixed(2) + 'B';
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(2) + 'K';
  if (Number.isInteger(n)) return n.toLocaleString();
  return n.toFixed(4);
}

function fmtPct(n) {
  if (n == null) return '-';
  var sign = n >= 0 ? '+' : '';
  return sign + n.toFixed(2) + '%';
}

function fmtAddr(addr) {
  if (!addr || addr.length < 12) return addr || '-';
  return addr.slice(0, 6) + '...' + addr.slice(-4);
}

function fmtTimestamp(ts) {
  if (!ts) return '-';
  var d = new Date(ts > 1e12 ? ts : ts * 1000);
  return d.toLocaleString();
}

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

// ===== Download Helpers =====
function downloadFile(filename, content, mimeType) {
  var blob = new Blob([content], { type: mimeType });
  var url = URL.createObjectURL(blob);
  var a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

function arrayToCSV(headers, rows) {
  var lines = [headers.join(',')];
  rows.forEach(function(row) {
    lines.push(row.map(function(cell) {
      var s = String(cell == null ? '' : cell);
      if (s.indexOf(',') >= 0 || s.indexOf('"') >= 0 || s.indexOf('\n') >= 0) {
        return '"' + s.replace(/"/g, '""') + '"';
      }
      return s;
    }).join(','));
  });
  return lines.join('\n');
}

function syntaxHighlightJSON(json) {
  return json.replace(
    /("(\\u[\da-fA-F]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g,
    function(match) {
      var cls = 'json-number';
      if (/^"/.test(match)) {
        cls = /:$/.test(match) ? 'json-key' : 'json-string';
      } else if (/true|false/.test(match)) {
        cls = 'json-boolean';
      } else if (/null/.test(match)) {
        cls = 'json-null';
      }
      return '<span class="' + cls + '">' + match + '</span>';
    }
  );
}

// ===== Common UI Components =====

function createDownloadBar(data, opts) {
  // opts: { csvFn, mdFn, filenameBase }
  opts = opts || {};
  var base = opts.filenameBase || 'scope-data';
  var jsonStr = JSON.stringify(data, null, 2);

  var bar = el('div', { className: 'download-bar' });
  bar.appendChild(el('span', { className: 'dl-label' }, 'Download'));

  // JSON
  bar.appendChild(el('button', {
    className: 'dl-btn',
    textContent: 'JSON',
    onClick: function() { downloadFile(base + '.json', jsonStr, 'application/json'); }
  }));

  // CSV
  if (opts.csvFn) {
    bar.appendChild(el('button', {
      className: 'dl-btn',
      textContent: 'CSV',
      onClick: function() { downloadFile(base + '.csv', opts.csvFn(data), 'text/csv'); }
    }));
  }

  // Markdown
  if (opts.mdFn) {
    bar.appendChild(el('button', {
      className: 'dl-btn',
      textContent: 'Markdown',
      onClick: function() { downloadFile(base + '.md', opts.mdFn(data), 'text/markdown'); }
    }));
  }

  bar.appendChild(el('span', { className: 'dl-spacer' }));

  // Raw JSON toggle
  var rawContainer = el('div', { className: 'raw-json-container' });
  var rawResults = el('div', { className: 'results' });
  var rawHeader = el('div', { className: 'results-header' });
  rawHeader.appendChild(el('span', null, 'Raw JSON'));
  var copyBtn = el('button', {
    className: 'btn btn-secondary',
    textContent: 'Copy',
    style: { padding: '4px 10px', fontSize: '11px' },
    onClick: function() {
      navigator.clipboard.writeText(jsonStr);
      copyBtn.textContent = 'Copied!';
      setTimeout(function() { copyBtn.textContent = 'Copy'; }, 1500);
    }
  });
  rawHeader.appendChild(copyBtn);
  rawResults.appendChild(rawHeader);
  rawResults.appendChild(el('pre', { innerHTML: syntaxHighlightJSON(jsonStr) }));
  rawContainer.appendChild(rawResults);

  var toggleBtn = el('button', {
    className: 'dl-toggle',
    textContent: 'View JSON',
    onClick: function() {
      var vis = rawContainer.classList.toggle('visible');
      toggleBtn.textContent = vis ? 'Hide JSON' : 'View JSON';
    }
  });
  bar.appendChild(toggleBtn);

  return { bar: bar, rawContainer: rawContainer };
}

function metricCard(label, value, sub) {
  var card = el('div', { className: 'metric-card' });
  card.appendChild(el('div', { className: 'metric-label', textContent: label }));
  if (typeof value === 'object' && value && value.nodeType) {
    var vDiv = el('div', { className: 'metric-value' });
    vDiv.appendChild(value);
    card.appendChild(vDiv);
  } else {
    card.appendChild(el('div', { className: 'metric-value', textContent: String(value) }));
  }
  if (sub) {
    var cls = 'metric-sub';
    if (sub.cls) cls += ' ' + sub.cls;
    card.appendChild(el('div', { className: cls, textContent: sub.text }));
  }
  return card;
}

function chainBadge(chain) {
  return el('span', { className: 'chain-badge', textContent: chain || 'unknown' });
}

function statusBadge(pass, label) {
  return el('span', {
    className: pass ? 'status-pass' : 'status-fail',
    textContent: label || (pass ? 'Success' : 'Failed')
  });
}

function buildDataTable(headers, rows, opts) {
  opts = opts || {};
  var wrap = el('div', { className: 'table-scroll' });
  var table = el('table', { className: 'data-table' });
  var thead = el('thead');
  var hr = el('tr');
  headers.forEach(function(h) {
    var th = el('th', null, typeof h === 'object' ? h.label : h);
    if ((typeof h === 'object' && h.right) || false) th.className = 'right';
    hr.appendChild(th);
  });
  thead.appendChild(hr);
  table.appendChild(thead);

  var tbody = el('tbody');
  rows.forEach(function(row) {
    var tr = el('tr');
    row.forEach(function(cell, idx) {
      var td = el('td');
      var hdr = headers[idx];
      if (typeof hdr === 'object' && hdr.right) td.className = 'right';
      if (typeof hdr === 'object' && hdr.addr) td.className = 'addr-cell';
      if (typeof cell === 'object' && cell && cell.nodeType) {
        td.appendChild(cell);
      } else {
        td.textContent = cell == null ? '-' : String(cell);
      }
      tr.appendChild(td);
    });
    tbody.appendChild(tr);
  });
  table.appendChild(tbody);
  wrap.appendChild(table);
  return wrap;
}

// ===== API Helpers =====
async function apiPost(endpoint, body, resultEl, renderer) {
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
    } else if (renderer) {
      clearElement(resultEl);
      renderer(resultEl, data);
    } else {
      showFallbackResults(resultEl, data);
    }
  } catch (e) {
    showError(resultEl, 'Request failed: ' + e.message);
  }
}

async function apiGet(endpoint, resultEl, renderer) {
  showLoading(resultEl);
  try {
    var res = await fetch(API + endpoint);
    var data = await res.json();
    if (data.error) {
      showError(resultEl, 'Error: ' + data.error);
    } else if (renderer) {
      clearElement(resultEl);
      renderer(resultEl, data);
    } else {
      showFallbackResults(resultEl, data);
    }
  } catch (e) {
    showError(resultEl, 'Request failed: ' + e.message);
  }
}

function showFallbackResults(resultEl, data) {
  clearElement(resultEl);
  var view = el('div', { className: 'result-view' });
  var dl = createDownloadBar(data, { filenameBase: 'scope-data' });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  // Show raw JSON expanded as fallback
  dl.rawContainer.classList.add('visible');
  resultEl.appendChild(view);
}

// =============================================================
//  RICH RENDERERS
// =============================================================

// ===== Address Renderer =====
function renderAddress(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Header
  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(chainBadge(data.chain));
  hdr.appendChild(el('span', { textContent: data.address || '-' }));
  var copyBtn = el('button', {
    className: 'copy-btn',
    textContent: 'Copy',
    onClick: function() {
      navigator.clipboard.writeText(data.address || '');
      copyBtn.textContent = 'Copied!';
      setTimeout(function() { copyBtn.textContent = 'Copy'; }, 1200);
    }
  });
  hdr.appendChild(copyBtn);
  view.appendChild(hdr);

  // Metrics
  var grid = el('div', { className: 'metric-grid' });
  grid.appendChild(metricCard('Balance',
    data.balance ? data.balance.formatted : '-',
    data.balance && data.balance.usd != null
      ? { text: fmtUsd(data.balance.usd), cls: 'muted' }
      : null
  ));
  grid.appendChild(metricCard('Transactions', String(data.transaction_count || 0)));
  if (data.tokens) {
    grid.appendChild(metricCard('Token Holdings', String(data.tokens.length)));
  }
  view.appendChild(grid);

  // Token Holdings table
  if (data.tokens && data.tokens.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Token Holdings'));
    var rows = data.tokens.map(function(t) {
      return [t.symbol, t.name, t.formatted_balance, el('span', { className: 'addr-cell', textContent: fmtAddr(t.contract_address) })];
    });
    view.appendChild(buildDataTable(
      ['Symbol', 'Name', 'Balance', { label: 'Contract', addr: true }],
      rows
    ));
  }

  // Transactions table
  if (data.transactions && data.transactions.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Recent Transactions'));
    var txRows = data.transactions.map(function(tx) {
      return [
        el('span', { className: 'addr-cell', textContent: fmtAddr(tx.hash) }),
        fmtAddr(tx.from),
        fmtAddr(tx.to || 'Contract'),
        tx.value,
        statusBadge(tx.status)
      ];
    });
    view.appendChild(buildDataTable(
      [{ label: 'Hash', addr: true }, 'From', 'To', 'Value', 'Status'],
      txRows
    ));
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-address-' + fmtAddr(data.address),
    csvFn: addressToCSV,
    mdFn: addressToMarkdown
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

function addressToCSV(data) {
  if (data.tokens && data.tokens.length > 0) {
    return arrayToCSV(
      ['symbol', 'name', 'balance', 'contract_address'],
      data.tokens.map(function(t) { return [t.symbol, t.name, t.formatted_balance, t.contract_address]; })
    );
  }
  return arrayToCSV(['address', 'chain', 'balance', 'usd', 'tx_count'], [
    [data.address, data.chain, data.balance ? data.balance.formatted : '', data.balance ? data.balance.usd : '', data.transaction_count]
  ]);
}

function addressToMarkdown(data) {
  var md = '# Address Analysis\n\n';
  md += '**Address:** `' + data.address + '`\n';
  md += '**Chain:** ' + data.chain + '\n\n';
  md += '## Balance\n\n';
  md += '- **Native:** ' + (data.balance ? data.balance.formatted : '-') + '\n';
  if (data.balance && data.balance.usd != null) md += '- **USD:** ' + fmtUsd(data.balance.usd) + '\n';
  md += '- **Transaction Count:** ' + (data.transaction_count || 0) + '\n\n';
  if (data.tokens && data.tokens.length > 0) {
    md += '## Token Holdings\n\n';
    md += '| Symbol | Name | Balance | Contract |\n|--------|------|---------|----------|\n';
    data.tokens.forEach(function(t) {
      md += '| ' + t.symbol + ' | ' + t.name + ' | ' + t.formatted_balance + ' | `' + t.contract_address + '` |\n';
    });
  }
  return md;
}

// ===== Transaction Renderer =====
function renderTransaction(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Header
  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(chainBadge(data.chain));
  hdr.appendChild(statusBadge(data.transaction && data.transaction.status));
  hdr.appendChild(el('span', { textContent: data.hash || '-', style: { flex: '1' } }));
  view.appendChild(hdr);

  var tx = data.transaction || {};
  var gas = data.gas || {};
  var block = data.block || {};

  // From → To flow
  var flow = el('div', { className: 'flow-display' });
  flow.appendChild(el('span', { className: 'flow-addr', textContent: tx.from || '-' }));
  flow.appendChild(el('span', { className: 'flow-arrow', textContent: '\u2192' }));
  flow.appendChild(el('span', { className: 'flow-addr', textContent: tx.to || 'Contract Creation' }));
  view.appendChild(flow);

  // Metrics
  var grid = el('div', { className: 'metric-grid' });
  grid.appendChild(metricCard('Value', tx.value || '0'));
  grid.appendChild(metricCard('Gas Fee', gas.transaction_fee || '-'));
  grid.appendChild(metricCard('Block', String(block.number || '-')));
  grid.appendChild(metricCard('Nonce', String(tx.nonce != null ? tx.nonce : '-')));
  if (block.timestamp) {
    grid.appendChild(metricCard('Timestamp', fmtTimestamp(block.timestamp)));
  }
  view.appendChild(grid);

  // Gas details
  view.appendChild(el('div', { className: 'section-title' }, 'Gas Details'));
  var gasGrid = el('div', { className: 'kv-grid' });
  [
    ['Gas Limit', fmtNum(gas.gas_limit)],
    ['Gas Used', fmtNum(gas.gas_used)],
    ['Gas Price', gas.gas_price || '-'],
    ['Effective Gas Price', gas.effective_gas_price || '-']
  ].forEach(function(pair) {
    gasGrid.appendChild(el('span', { className: 'kv-key', textContent: pair[0] }));
    gasGrid.appendChild(el('span', { className: 'kv-val', textContent: pair[1] }));
  });
  view.appendChild(gasGrid);

  // Decoded Input
  if (data.decoded_input) {
    view.appendChild(el('div', { className: 'section-title' }, 'Decoded Input'));
    var di = data.decoded_input;
    var diHeader = el('div', { className: 'result-header', style: { marginBottom: '8px' } });
    diHeader.appendChild(el('span', {
      textContent: (di.function_name || di.function_signature || 'Unknown'),
      style: { color: 'var(--accent)', fontWeight: '600' }
    }));
    view.appendChild(diHeader);
    if (di.parameters && di.parameters.length > 0) {
      var paramRows = di.parameters.map(function(p) {
        return [p.name || '-', p.param_type || '-', p.value || '-'];
      });
      view.appendChild(buildDataTable(['Name', 'Type', 'Value'], paramRows));
    }
  }

  // Internal Transactions
  if (data.internal_transactions && data.internal_transactions.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Internal Transactions (' + data.internal_transactions.length + ')'));
    var itRows = data.internal_transactions.map(function(it) {
      return [it.call_type || '-', fmtAddr(it.from), fmtAddr(it.to), it.value || '0'];
    });
    view.appendChild(buildDataTable(['Type', 'From', 'To', 'Value'], itRows));
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-tx-' + fmtAddr(data.hash),
    csvFn: txToCSV,
    mdFn: txToMarkdown
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

function txToCSV(data) {
  var tx = data.transaction || {};
  var gas = data.gas || {};
  return arrayToCSV(
    ['hash', 'chain', 'from', 'to', 'value', 'status', 'block', 'gas_used', 'fee'],
    [[data.hash, data.chain, tx.from, tx.to, tx.value, tx.status, (data.block || {}).number, gas.gas_used, gas.transaction_fee]]
  );
}

function txToMarkdown(data) {
  var tx = data.transaction || {};
  var gas = data.gas || {};
  var md = '# Transaction Analysis\n\n';
  md += '**Hash:** `' + data.hash + '`\n';
  md += '**Chain:** ' + data.chain + '\n';
  md += '**Status:** ' + (tx.status ? 'Success' : 'Failed') + '\n\n';
  md += '## Details\n\n';
  md += '- **From:** `' + (tx.from || '-') + '`\n';
  md += '- **To:** `' + (tx.to || 'Contract Creation') + '`\n';
  md += '- **Value:** ' + (tx.value || '0') + '\n';
  md += '- **Fee:** ' + (gas.transaction_fee || '-') + '\n';
  md += '- **Block:** ' + ((data.block || {}).number || '-') + '\n';
  return md;
}

// ===== Token Crawl Renderer =====
function renderCrawl(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Header
  var token = data.token || {};
  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(chainBadge(data.chain));
  hdr.appendChild(el('span', { textContent: token.symbol || '?', style: { fontWeight: '700', fontSize: '16px' } }));
  hdr.appendChild(el('span', { textContent: token.name || '', style: { color: 'var(--text-muted)' } }));
  view.appendChild(hdr);

  // Metrics
  var grid = el('div', { className: 'metric-grid' });
  var changeVal = data.price_change_24h;
  grid.appendChild(metricCard('Price', fmtPrice(data.price_usd),
    changeVal != null ? { text: fmtPct(changeVal) + ' (24h)', cls: changeVal >= 0 ? 'positive' : 'negative' } : null
  ));
  grid.appendChild(metricCard('Volume (24h)', fmtUsd(data.volume_24h)));
  grid.appendChild(metricCard('Liquidity', fmtUsd(data.liquidity_usd)));
  grid.appendChild(metricCard('Market Cap', fmtUsd(data.market_cap)));
  if (data.fdv) grid.appendChild(metricCard('FDV', fmtUsd(data.fdv)));
  if (data.total_holders) grid.appendChild(metricCard('Total Holders', fmtNum(data.total_holders)));
  if (data.top_10_concentration != null) {
    grid.appendChild(metricCard('Top 10 Conc.', data.top_10_concentration.toFixed(1) + '%',
      { text: data.top_10_concentration > 50 ? 'High concentration' : 'Healthy distribution', cls: data.top_10_concentration > 50 ? 'negative' : 'positive' }
    ));
  }
  view.appendChild(grid);

  // DEX Pairs
  if (data.dex_pairs && data.dex_pairs.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Trading Pairs (' + data.dex_pairs.length + ')'));
    var pairRows = data.dex_pairs.map(function(p) {
      var chg = p.price_change_24h;
      var chgEl = el('span', {
        textContent: fmtPct(chg),
        style: { color: chg >= 0 ? 'var(--green)' : 'var(--red)' }
      });
      return [
        p.dex_name || '-',
        (p.base_token || '') + '/' + (p.quote_token || ''),
        fmtPrice(p.price_usd),
        fmtUsd(p.volume_24h),
        fmtUsd(p.liquidity_usd),
        chgEl,
        String((p.buys_24h || 0)) + '/' + String((p.sells_24h || 0))
      ];
    });
    view.appendChild(buildDataTable(
      ['DEX', 'Pair', { label: 'Price', right: true }, { label: 'Vol 24h', right: true },
       { label: 'Liquidity', right: true }, { label: '24h Chg', right: true }, { label: 'B/S', right: true }],
      pairRows
    ));
  }

  // Top Holders
  if (data.holders && data.holders.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Top Holders'));
    var holderRows = data.holders.map(function(h) {
      return [
        '#' + h.rank,
        el('span', { className: 'addr-cell', textContent: fmtAddr(h.address) }),
        h.formatted_balance || h.balance,
        h.percentage.toFixed(2) + '%'
      ];
    });
    view.appendChild(buildDataTable(
      ['Rank', { label: 'Address', addr: true }, 'Balance', { label: '% Supply', right: true }],
      holderRows
    ));
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-crawl-' + (token.symbol || 'token'),
    csvFn: crawlToCSV,
    mdFn: crawlToMarkdown
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

function crawlToCSV(data) {
  if (data.dex_pairs && data.dex_pairs.length > 0) {
    return arrayToCSV(
      ['dex', 'pair', 'price_usd', 'volume_24h', 'liquidity_usd', 'price_change_24h', 'buys_24h', 'sells_24h'],
      data.dex_pairs.map(function(p) {
        return [p.dex_name, p.base_token + '/' + p.quote_token, p.price_usd, p.volume_24h, p.liquidity_usd, p.price_change_24h, p.buys_24h, p.sells_24h];
      })
    );
  }
  return arrayToCSV(['symbol', 'price', 'volume_24h', 'liquidity', 'market_cap'], [
    [(data.token || {}).symbol, data.price_usd, data.volume_24h, data.liquidity_usd, data.market_cap]
  ]);
}

function crawlToMarkdown(data) {
  var token = data.token || {};
  var md = '# Token Crawl: ' + token.symbol + '\n\n';
  md += '**Token:** ' + token.name + ' (' + token.symbol + ')\n';
  md += '**Chain:** ' + data.chain + '\n';
  md += '**Contract:** `' + token.contract_address + '`\n\n';
  md += '## Key Metrics\n\n';
  md += '| Metric | Value |\n|--------|-------|\n';
  md += '| Price | ' + fmtPrice(data.price_usd) + ' |\n';
  md += '| 24h Change | ' + fmtPct(data.price_change_24h) + ' |\n';
  md += '| Volume (24h) | ' + fmtUsd(data.volume_24h) + ' |\n';
  md += '| Liquidity | ' + fmtUsd(data.liquidity_usd) + ' |\n';
  md += '| Market Cap | ' + fmtUsd(data.market_cap) + ' |\n';
  if (data.dex_pairs && data.dex_pairs.length > 0) {
    md += '\n## Trading Pairs\n\n';
    md += '| DEX | Pair | Price | Volume | Liquidity |\n|-----|------|-------|--------|----------|\n';
    data.dex_pairs.forEach(function(p) {
      md += '| ' + p.dex_name + ' | ' + p.base_token + '/' + p.quote_token + ' | ' + fmtPrice(p.price_usd) + ' | ' + fmtUsd(p.volume_24h) + ' | ' + fmtUsd(p.liquidity_usd) + ' |\n';
    });
  }
  return md;
}

// ===== Market Summary Renderer =====
function renderMarketSummary(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Header
  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(el('span', { textContent: data.pair || '?', style: { fontWeight: '700', fontSize: '16px' } }));
  hdr.appendChild(statusBadge(data.healthy, data.healthy ? 'Healthy' : 'Unhealthy'));
  view.appendChild(hdr);

  // Metrics
  var grid = el('div', { className: 'metric-grid' });
  grid.appendChild(metricCard('Best Bid', formatPrice(data.best_bid)));
  grid.appendChild(metricCard('Best Ask', formatPrice(data.best_ask)));
  grid.appendChild(metricCard('Mid Price', formatPrice(data.mid_price)));
  grid.appendChild(metricCard('Spread', formatPrice(data.spread)));
  if (data.volume_24h != null) grid.appendChild(metricCard('Volume (24h)', fmtUsd(data.volume_24h)));
  grid.appendChild(metricCard('Bid Depth', fmtUsd(data.bid_depth)));
  grid.appendChild(metricCard('Ask Depth', fmtUsd(data.ask_depth)));
  view.appendChild(grid);

  // Execution simulation
  if (data.execution_10k_buy || data.execution_10k_sell) {
    view.appendChild(el('div', { className: 'section-title' }, 'Execution Simulation ($10K)'));
    var execGrid = el('div', { className: 'metric-grid' });
    if (data.execution_10k_buy) {
      var b = data.execution_10k_buy;
      execGrid.appendChild(metricCard('Buy VWAP', formatPrice(b.vwap),
        { text: b.slippage_bps.toFixed(1) + ' bps slippage', cls: b.slippage_bps > 10 ? 'negative' : 'positive' }
      ));
    }
    if (data.execution_10k_sell) {
      var s = data.execution_10k_sell;
      execGrid.appendChild(metricCard('Sell VWAP', formatPrice(s.vwap),
        { text: s.slippage_bps.toFixed(1) + ' bps slippage', cls: s.slippage_bps > 10 ? 'negative' : 'positive' }
      ));
    }
    view.appendChild(execGrid);
  }

  // Health checks
  if (data.checks && data.checks.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Health Checks'));
    var checkList = el('div', { className: 'check-list' });
    data.checks.forEach(function(c) {
      var pass = c.status === 'pass';
      var item = el('div', { className: 'check-item ' + (pass ? 'pass' : 'fail') });
      item.appendChild(el('span', { className: 'check-icon', textContent: pass ? '\u2713' : '\u2717' }));
      item.appendChild(el('span', { textContent: c.message }));
      checkList.appendChild(item);
    });
    view.appendChild(checkList);
  }

  // Order book (reuse exchange renderer components)
  if ((data.bids && data.bids.length > 0) || (data.asks && data.asks.length > 0)) {
    view.appendChild(el('div', { className: 'section-title' }, 'Order Book'));
    renderOrderBook(view, data);
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-market-' + (data.pair || 'summary'),
    csvFn: marketToCSV,
    mdFn: marketToMarkdown
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

function marketToCSV(data) {
  var headers = ['side', 'price', 'quantity', 'value'];
  var rows = [];
  (data.asks || []).forEach(function(a) { rows.push(['ask', a.price, a.quantity, a.value]); });
  (data.bids || []).forEach(function(b) { rows.push(['bid', b.price, b.quantity, b.value]); });
  return arrayToCSV(headers, rows);
}

function marketToMarkdown(data) {
  var md = '# Market Summary: ' + (data.pair || '') + '\n\n';
  md += '| Metric | Value |\n|--------|-------|\n';
  md += '| Best Bid | ' + formatPrice(data.best_bid) + ' |\n';
  md += '| Best Ask | ' + formatPrice(data.best_ask) + ' |\n';
  md += '| Spread | ' + formatPrice(data.spread) + ' |\n';
  md += '| Mid Price | ' + formatPrice(data.mid_price) + ' |\n';
  md += '| Healthy | ' + (data.healthy ? 'Yes' : 'No') + ' |\n';
  if (data.checks && data.checks.length > 0) {
    md += '\n## Health Checks\n\n';
    data.checks.forEach(function(c) {
      md += '- [' + (c.status === 'pass' ? 'x' : ' ') + '] ' + c.message + '\n';
    });
  }
  return md;
}

// ===== Token Health Renderer =====
function renderTokenHealth(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Render analytics portion
  if (data.analytics) {
    var a = data.analytics;
    var token = a.token || {};
    var hdr = el('div', { className: 'result-header' });
    hdr.appendChild(chainBadge(a.chain));
    hdr.appendChild(el('span', { textContent: token.symbol || '?', style: { fontWeight: '700', fontSize: '16px' } }));
    hdr.appendChild(el('span', { textContent: token.name || '', style: { color: 'var(--text-muted)' } }));
    view.appendChild(hdr);

    var grid = el('div', { className: 'metric-grid' });
    grid.appendChild(metricCard('Price', fmtPrice(a.price_usd)));
    grid.appendChild(metricCard('Volume (24h)', fmtUsd(a.volume_24h)));
    grid.appendChild(metricCard('Liquidity', fmtUsd(a.liquidity_usd)));
    grid.appendChild(metricCard('Market Cap', fmtUsd(a.market_cap)));
    if (a.top_10_concentration != null) {
      grid.appendChild(metricCard('Top 10 Conc.', a.top_10_concentration.toFixed(1) + '%'));
    }
    view.appendChild(grid);
  }

  // Market health portion
  if (data.market) {
    var m = data.market;
    view.appendChild(el('div', { className: 'section-title' }, 'Market Health'));
    var mHdr = el('div', { className: 'result-header', style: { marginBottom: '12px' } });
    mHdr.appendChild(el('span', { textContent: m.pair || '-', style: { fontWeight: '600' } }));
    mHdr.appendChild(statusBadge(m.healthy, m.healthy ? 'Healthy' : 'Unhealthy'));
    view.appendChild(mHdr);

    var mGrid = el('div', { className: 'metric-grid' });
    mGrid.appendChild(metricCard('Bid', formatPrice(m.best_bid)));
    mGrid.appendChild(metricCard('Ask', formatPrice(m.best_ask)));
    mGrid.appendChild(metricCard('Spread', formatPrice(m.spread)));
    mGrid.appendChild(metricCard('Bid Depth', fmtUsd(m.bid_depth)));
    mGrid.appendChild(metricCard('Ask Depth', fmtUsd(m.ask_depth)));
    view.appendChild(mGrid);

    if (m.checks && m.checks.length > 0) {
      var checkList = el('div', { className: 'check-list' });
      m.checks.forEach(function(c) {
        var pass = c.status === 'pass';
        var item = el('div', { className: 'check-item ' + (pass ? 'pass' : 'fail') });
        item.appendChild(el('span', { className: 'check-icon', textContent: pass ? '\u2713' : '\u2717' }));
        item.appendChild(el('span', { textContent: c.message }));
        checkList.appendChild(item);
      });
      view.appendChild(checkList);
    }
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-token-health-' + ((data.analytics || {}).token || {}).symbol,
    csvFn: function(d) {
      if (d.analytics) return crawlToCSV(d.analytics);
      return JSON.stringify(d);
    },
    mdFn: function(d) {
      var md = '# Token Health Report\n\n';
      if (d.analytics) md += crawlToMarkdown(d.analytics);
      if (d.market) {
        md += '\n## Market Health\n\n';
        md += '- **Pair:** ' + d.market.pair + '\n';
        md += '- **Healthy:** ' + (d.market.healthy ? 'Yes' : 'No') + '\n';
        md += '- **Spread:** ' + formatPrice(d.market.spread) + '\n';
      }
      return md;
    }
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

// ===== Discover Renderer =====
function renderDiscover(resultEl, data) {
  var items = Array.isArray(data) ? data : (data.tokens || []);
  var view = el('div', { className: 'result-view' });

  view.appendChild(el('div', { className: 'section-title' }, 'Discovered Tokens (' + items.length + ')'));

  if (items.length === 0) {
    view.appendChild(el('div', { className: 'empty-state' }, 'No tokens found for this query.'));
  } else {
    var grid = el('div', { className: 'discover-grid' });
    items.forEach(function(item) {
      var card = el('div', { className: 'discover-card' });
      var header = el('div', { className: 'dc-header' });
      header.appendChild(chainBadge(item.chain_id || item.chain || '?'));
      card.appendChild(header);
      if (item.token_address) {
        card.appendChild(el('div', { className: 'dc-addr', textContent: item.token_address }));
      }
      if (item.description) {
        card.appendChild(el('div', { className: 'dc-desc', textContent: item.description }));
      }
      if (item.links && item.links.length > 0) {
        var linksDiv = el('div', { className: 'dc-links' });
        item.links.forEach(function(link) {
          linksDiv.appendChild(el('a', {
            className: 'dc-link',
            href: link.url,
            target: '_blank',
            rel: 'noopener',
            textContent: link.label || link.link_type || 'Link'
          }));
        });
        card.appendChild(linksDiv);
      }
      if (item.url) {
        var urlDiv = el('div', { className: 'dc-links', style: { marginTop: '6px' } });
        urlDiv.appendChild(el('a', {
          className: 'dc-link',
          href: item.url,
          target: '_blank',
          rel: 'noopener',
          textContent: 'DexScreener'
        }));
        card.appendChild(urlDiv);
      }
      grid.appendChild(card);
    });
    view.appendChild(grid);
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-discover',
    csvFn: function(d) {
      var arr = Array.isArray(d) ? d : (d.tokens || []);
      return arrayToCSV(
        ['chain', 'token_address', 'url', 'description'],
        arr.map(function(i) { return [i.chain_id || i.chain, i.token_address, i.url, i.description]; })
      );
    },
    mdFn: function(d) {
      var arr = Array.isArray(d) ? d : (d.tokens || []);
      var md = '# Token Discovery\n\n';
      arr.forEach(function(i, idx) {
        md += '## ' + (idx + 1) + '. ' + (i.chain_id || i.chain || '?') + '\n\n';
        md += '- **Address:** `' + (i.token_address || '-') + '`\n';
        if (i.url) md += '- **URL:** ' + i.url + '\n';
        if (i.description) md += '- **Description:** ' + i.description + '\n';
        md += '\n';
      });
      return md;
    }
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

// ===== Contract Analysis =====
function runContract() {
  var address = document.getElementById('ct-address').value.trim();
  if (!address) return;
  var chain = document.getElementById('ct-chain').value;
  apiPost('/api/contract', { address: address, chain: chain },
    document.getElementById('contract-results'), renderContract);
}

function renderContract(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Header
  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(chainBadge(data.chain));
  hdr.appendChild(el('span', { className: 'addr-cell', textContent: data.address }));
  if (data.source_info) {
    hdr.appendChild(el('span', { className: 'ct-name-badge', textContent: data.source_info.contract_name }));
  }
  hdr.appendChild(el('span', {
    className: data.is_verified ? 'ct-verified-badge' : 'ct-unverified-badge',
    textContent: data.is_verified ? 'Verified' : 'Unverified'
  }));
  view.appendChild(hdr);

  // Security Score
  var scoreSection = el('div', { className: 'ct-score-section' });
  var score = data.security_score || 0;
  var scoreClass = score >= 80 ? 'ct-score-good' : score >= 60 ? 'ct-score-moderate' : score >= 40 ? 'ct-score-caution' : 'ct-score-danger';
  var scoreCircle = el('div', { className: 'ct-score-circle ' + scoreClass, textContent: score });
  scoreSection.appendChild(scoreCircle);
  var scoreInfo = el('div', { className: 'ct-score-info' });
  var label = score >= 80 ? 'GOOD' : score >= 60 ? 'MODERATE' : score >= 40 ? 'CAUTION' : score >= 20 ? 'HIGH RISK' : 'CRITICAL';
  scoreInfo.appendChild(el('div', { className: 'ct-score-label', textContent: 'Security Score: ' + label }));
  var barOuter = el('div', { className: 'ct-score-bar-outer' });
  barOuter.appendChild(el('div', { className: 'ct-score-bar-fill ' + scoreClass, style: { width: score + '%' } }));
  scoreInfo.appendChild(barOuter);
  scoreInfo.appendChild(el('div', { className: 'ct-score-summary', textContent: data.security_summary || '' }));
  scoreSection.appendChild(scoreInfo);
  view.appendChild(scoreSection);

  // Source Info
  if (data.source_info) {
    view.appendChild(el('div', { className: 'section-title' }, 'Source Code'));
    var src = data.source_info;
    var srcGrid = el('div', { className: 'metric-grid' });
    srcGrid.appendChild(metricCard('Contract', src.contract_name));
    srcGrid.appendChild(metricCard('Compiler', src.compiler_version));
    srcGrid.appendChild(metricCard('EVM', src.evm_version || '-'));
    srcGrid.appendChild(metricCard('License', src.license_type || '-'));
    srcGrid.appendChild(metricCard('Optimization', src.optimization_used ? src.optimization_runs + ' runs' : 'Off'));
    srcGrid.appendChild(metricCard('ABI Functions', src.parsed_abi ? src.parsed_abi.length : 0));
    view.appendChild(srcGrid);
  }

  // Proxy Detection
  if (data.proxy_info) {
    view.appendChild(el('div', { className: 'section-title' }, 'Proxy Detection'));
    var px = data.proxy_info;
    if (px.is_proxy) {
      var pxGrid = el('div', { className: 'metric-grid' });
      pxGrid.appendChild(metricCard('Proxy Type', px.proxy_type));
      if (px.implementation_address) pxGrid.appendChild(metricCard('Implementation', fmtAddr(px.implementation_address)));
      if (px.admin_address) pxGrid.appendChild(metricCard('Admin', fmtAddr(px.admin_address)));
      if (px.beacon_address) pxGrid.appendChild(metricCard('Beacon', fmtAddr(px.beacon_address)));
      view.appendChild(pxGrid);
    } else {
      view.appendChild(el('div', { className: 'ct-note', textContent: 'Not a proxy contract.' }));
    }
    if (px.details && px.details.length > 0) {
      var detList = el('ul', { className: 'ct-detail-list' });
      px.details.forEach(function(d) { detList.appendChild(el('li', { textContent: d })); });
      view.appendChild(detList);
    }
  }

  // Access Control
  if (data.access_control) {
    view.appendChild(el('div', { className: 'section-title' }, 'Access Control'));
    var ac = data.access_control;
    var acGrid = el('div', { className: 'metric-grid' });
    acGrid.appendChild(metricCard('Ownership', ac.ownership_pattern || 'None detected'));
    acGrid.appendChild(metricCard('Renounced', ac.has_renounced_ownership ? 'Yes' : 'No'));
    acGrid.appendChild(metricCard('Role-Based', ac.has_role_based_access ? 'Yes' : 'No'));
    acGrid.appendChild(metricCard('tx.origin', ac.uses_tx_origin ? 'DANGER' : 'Safe', ac.uses_tx_origin ? { text: 'Vulnerable to phishing', cls: 'red' } : null));
    view.appendChild(acGrid);

    if (ac.roles && ac.roles.length > 0) {
      var rolesDiv = el('div', { className: 'ct-tags' });
      rolesDiv.appendChild(el('strong', { textContent: 'Roles: ' }));
      ac.roles.forEach(function(r) {
        rolesDiv.appendChild(el('span', { className: 'ct-tag', textContent: r }));
      });
      view.appendChild(rolesDiv);
    }

    if (ac.privileged_functions && ac.privileged_functions.length > 0) {
      view.appendChild(el('div', { className: 'section-subtitle' }, 'Privileged Functions'));
      var pfRows = ac.privileged_functions.map(function(pf) {
        var riskCls = (pf.risk || '').toLowerCase();
        return [
          pf.name,
          pf.modifiers ? pf.modifiers.join(', ') : '-',
          pf.capability,
          el('span', { className: 'ct-risk-badge ct-risk-' + riskCls, textContent: pf.risk || '-' })
        ];
      });
      view.appendChild(buildDataTable(['Function', 'Modifier', 'Capability', 'Risk'], pfRows));
    }

    if (ac.auth_analysis) {
      view.appendChild(el('div', { className: 'ct-auth-summary', textContent: ac.auth_analysis.summary }));
    }
  }

  // Vulnerabilities
  view.appendChild(el('div', { className: 'section-title' }, 'Vulnerability Scan'));
  if (data.vulnerabilities && data.vulnerabilities.length > 0) {
    var vulnContainer = el('div', { className: 'ct-vuln-list' });
    data.vulnerabilities.forEach(function(v) {
      var sevCls = (v.severity || 'informational').toLowerCase();
      var card = el('div', { className: 'ct-vuln-card ct-sev-' + sevCls });
      var cardHdr = el('div', { className: 'ct-vuln-header' });
      cardHdr.appendChild(el('span', { className: 'ct-sev-badge ct-sev-' + sevCls, textContent: v.severity }));
      cardHdr.appendChild(el('span', { className: 'ct-vuln-id', textContent: v.id }));
      cardHdr.appendChild(el('span', { className: 'ct-vuln-title', textContent: v.title }));
      card.appendChild(cardHdr);
      card.appendChild(el('div', { className: 'ct-vuln-desc', textContent: v.description }));
      card.appendChild(el('div', { className: 'ct-vuln-fix', textContent: 'Fix: ' + v.recommendation }));
      vulnContainer.appendChild(card);
    });
    view.appendChild(vulnContainer);
  } else {
    view.appendChild(el('div', { className: 'ct-note ct-note-good', textContent: 'No vulnerability heuristics triggered.' }));
  }

  // DeFi Analysis
  if (data.defi_analysis) {
    view.appendChild(el('div', { className: 'section-title' }, 'DeFi Analysis'));
    var df = data.defi_analysis;
    var dfGrid = el('div', { className: 'metric-grid' });
    dfGrid.appendChild(metricCard('Protocol Type', df.protocol_type || '-'));
    if (df.token_standards && df.token_standards.length > 0) {
      dfGrid.appendChild(metricCard('Token Standards', df.token_standards.join(', ')));
    }
    dfGrid.appendChild(metricCard('Oracle', df.has_oracle_dependency ? 'Yes' : 'No'));
    dfGrid.appendChild(metricCard('Flash Loan Risk', df.has_flash_loan_risk ? 'Yes' : 'No'));
    view.appendChild(dfGrid);

    if (df.oracle_info && df.oracle_info.length > 0) {
      view.appendChild(el('div', { className: 'section-subtitle' }, 'Oracle Dependencies'));
      df.oracle_info.forEach(function(o) {
        var oCard = el('div', { className: 'ct-oracle-card' });
        oCard.appendChild(el('strong', { textContent: o.provider + ': ' }));
        oCard.appendChild(el('span', { textContent: o.usage }));
        if (o.risks && o.risks.length > 0) {
          var rList = el('ul', { className: 'ct-detail-list' });
          o.risks.forEach(function(r) { rList.appendChild(el('li', { className: 'ct-warn', textContent: r })); });
          oCard.appendChild(rList);
        }
        view.appendChild(oCard);
      });
    }

    if (df.dex_integrations && df.dex_integrations.length > 0) {
      view.appendChild(el('div', { className: 'section-subtitle' }, 'DEX Integrations'));
      var dexRows = df.dex_integrations.map(function(d) {
        return [
          d.dex,
          d.integration_type,
          el('span', { className: d.has_slippage_protection ? 'ct-check-ok' : 'ct-check-fail', textContent: d.has_slippage_protection ? 'Yes' : 'NO' }),
          el('span', { className: d.has_deadline_protection ? 'ct-check-ok' : 'ct-check-fail', textContent: d.has_deadline_protection ? 'Yes' : 'NO' })
        ];
      });
      view.appendChild(buildDataTable(['DEX', 'Type', 'Slippage', 'Deadline'], dexRows));
    }

    if (df.risk_factors && df.risk_factors.length > 0) {
      view.appendChild(el('div', { className: 'section-subtitle' }, 'DeFi Risk Factors'));
      df.risk_factors.forEach(function(rf) {
        var rfDiv = el('div', { className: 'ct-risk-factor' });
        rfDiv.appendChild(el('span', { className: 'ct-rf-name', textContent: rf.name }));
        rfDiv.appendChild(el('span', { className: 'ct-rf-sev', textContent: rf.severity + '/10' }));
        rfDiv.appendChild(el('div', { className: 'ct-rf-desc', textContent: rf.description }));
        view.appendChild(rfDiv);
      });
    }
  }

  // External Intelligence
  if (data.external_info) {
    view.appendChild(el('div', { className: 'section-title' }, 'External Intelligence'));
    var ext = data.external_info;
    var extGrid = el('div', { className: 'metric-grid' });
    if (ext.github_repo) {
      var ghLink = el('a', { href: ext.github_repo, target: '_blank', textContent: ext.github_repo, className: 'ct-link' });
      extGrid.appendChild(metricCard('GitHub', ghLink));
    }
    if (ext.sourcify_verified != null) {
      extGrid.appendChild(metricCard('Sourcify', ext.sourcify_verified ? 'Verified' : 'Not verified'));
    }
    extGrid.appendChild(metricCard('Explorer', el('a', { href: ext.explorer_url, target: '_blank', textContent: 'View', className: 'ct-link' })));
    view.appendChild(extGrid);

    if (ext.audit_reports && ext.audit_reports.length > 0) {
      view.appendChild(el('div', { className: 'section-subtitle' }, 'Audit Reports'));
      var auditRows = ext.audit_reports.map(function(a) {
        var link = a.url ? el('a', { href: a.url, target: '_blank', textContent: 'Link', className: 'ct-link' }) : '-';
        return [a.auditor, a.scope, a.date || '-', link];
      });
      view.appendChild(buildDataTable(['Auditor', 'Scope', 'Date', 'Report'], auditRows));
    }

    if (ext.metadata && ext.metadata.length > 0) {
      view.appendChild(el('div', { className: 'section-subtitle' }, 'Metadata'));
      var metaRows = ext.metadata.map(function(m) { return [m.key, m.value]; });
      view.appendChild(buildDataTable(['Key', 'Value'], metaRows));
    }
  }

  // Download bar
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-contract-' + fmtAddr(data.address),
    csvFn: function(d) {
      var rows = [];
      if (d.vulnerabilities) {
        rows = d.vulnerabilities.map(function(v) {
          return [v.id, v.severity, v.category, v.title, v.description, v.recommendation];
        });
        return arrayToCSV(['id', 'severity', 'category', 'title', 'description', 'recommendation'], rows);
      }
      return JSON.stringify(d);
    },
    mdFn: function(d) {
      var md = '# Contract Analysis Report\n\n';
      md += '**Address:** `' + d.address + '`\n';
      md += '**Chain:** ' + d.chain + '\n';
      md += '**Verified:** ' + (d.is_verified ? 'Yes' : 'No') + '\n';
      md += '**Security Score:** ' + d.security_score + '/100\n\n';
      md += d.security_summary + '\n\n';
      if (d.source_info) {
        md += '## Source\n\n';
        md += '- **Name:** ' + d.source_info.contract_name + '\n';
        md += '- **Compiler:** ' + d.source_info.compiler_version + '\n';
        md += '- **License:** ' + (d.source_info.license_type || '-') + '\n\n';
      }
      if (d.proxy_info && d.proxy_info.is_proxy) {
        md += '## Proxy\n\n';
        md += '- **Type:** ' + d.proxy_info.proxy_type + '\n';
        if (d.proxy_info.implementation_address) md += '- **Implementation:** `' + d.proxy_info.implementation_address + '`\n';
        md += '\n';
      }
      if (d.access_control) {
        md += '## Access Control\n\n';
        md += '- **Ownership:** ' + (d.access_control.ownership_pattern || 'None') + '\n';
        md += '- **Renounced:** ' + (d.access_control.has_renounced_ownership ? 'Yes' : 'No') + '\n';
        md += '- **tx.origin:** ' + (d.access_control.uses_tx_origin ? 'DANGER' : 'Safe') + '\n\n';
      }
      if (d.vulnerabilities && d.vulnerabilities.length > 0) {
        md += '## Vulnerabilities\n\n';
        md += '| ID | Severity | Title | Recommendation |\n|-----|----------|-------|----------------|\n';
        d.vulnerabilities.forEach(function(v) {
          md += '| ' + v.id + ' | ' + v.severity + ' | ' + v.title + ' | ' + v.recommendation + ' |\n';
        });
        md += '\n';
      }
      return md;
    }
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

// ===== Compliance Risk Renderer =====
function renderCompliance(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  // Header
  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(chainBadge(data.chain));
  hdr.appendChild(el('span', { textContent: data.address || '-' }));
  view.appendChild(hdr);

  // Risk display
  var score = data.overall_score || 0;
  var level = (data.risk_level || 'unknown').toLowerCase();
  var riskDisp = el('div', { className: 'risk-display' });
  var circleClass = 'risk-score-circle risk-' + level;
  riskDisp.appendChild(el('div', { className: circleClass, textContent: score.toFixed(1) }));
  var riskInfo = el('div', { className: 'risk-info' });
  riskInfo.appendChild(el('div', { className: 'risk-level-text', textContent: (data.risk_level || 'Unknown') + ' Risk' }));
  riskInfo.appendChild(el('div', { className: 'risk-summary', textContent: 'Overall score: ' + score.toFixed(1) + '/10' }));
  riskDisp.appendChild(riskInfo);
  view.appendChild(riskDisp);

  // Factor breakdown
  if (data.factors && data.factors.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Risk Factors'));
    var factorsDiv = el('div');
    data.factors.forEach(function(f) {
      var row = el('div', { className: 'factor-row' });
      row.appendChild(el('span', { className: 'factor-name', textContent: f.name }));
      var barBg = el('div', { className: 'factor-bar-bg' });
      var pct = Math.min((f.score / 10) * 100, 100);
      var color = f.score <= 3 ? 'var(--green)' : f.score <= 6 ? 'var(--orange)' : 'var(--red)';
      barBg.appendChild(el('div', {
        className: 'factor-bar-fill',
        style: { width: pct + '%', background: color }
      }));
      row.appendChild(barBg);
      row.appendChild(el('span', { className: 'factor-score', textContent: f.score.toFixed(1) }));
      factorsDiv.appendChild(row);
    });
    view.appendChild(factorsDiv);
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-compliance-' + fmtAddr(data.address),
    csvFn: function(d) {
      if (d.factors) {
        return arrayToCSV(
          ['factor', 'score', 'weight', 'description'],
          d.factors.map(function(f) { return [f.name, f.score, f.weight, f.description]; })
        );
      }
      return JSON.stringify(d);
    },
    mdFn: function(d) {
      var md = '# Compliance Risk Assessment\n\n';
      md += '**Address:** `' + d.address + '`\n';
      md += '**Chain:** ' + d.chain + '\n';
      md += '**Risk Level:** ' + d.risk_level + ' (' + d.overall_score.toFixed(1) + '/10)\n\n';
      if (d.factors) {
        md += '## Factors\n\n';
        md += '| Factor | Score | Weight | Description |\n|--------|-------|--------|-------------|\n';
        d.factors.forEach(function(f) {
          md += '| ' + f.name + ' | ' + f.score.toFixed(1) + ' | ' + f.weight + ' | ' + f.description + ' |\n';
        });
      }
      return md;
    }
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

// ===== Export Renderer =====
function renderExport(resultEl, data) {
  var view = el('div', { className: 'result-view' });

  var hdr = el('div', { className: 'result-header' });
  hdr.appendChild(chainBadge(data.chain));
  hdr.appendChild(el('span', { textContent: data.address || '-' }));
  view.appendChild(hdr);

  var grid = el('div', { className: 'metric-grid' });
  if (data.balance) {
    grid.appendChild(metricCard('Balance', data.balance.formatted || data.balance.raw || '-',
      data.balance.usd_value != null ? { text: fmtUsd(data.balance.usd_value), cls: 'muted' } : null
    ));
  }
  if (data.transactions) {
    grid.appendChild(metricCard('Transactions', String(data.transactions.length)));
  }
  if (data.tokens) {
    grid.appendChild(metricCard('Tokens', String(data.tokens.length)));
  }
  view.appendChild(grid);

  // Token table
  if (data.tokens && data.tokens.length > 0) {
    view.appendChild(el('div', { className: 'section-title' }, 'Token Balances'));
    var rows = data.tokens.map(function(t) {
      return [t.symbol || '-', t.name || '-', t.formatted_balance || t.balance || '-', fmtAddr(t.contract_address)];
    });
    view.appendChild(buildDataTable(['Symbol', 'Name', 'Balance', 'Contract'], rows));
  }

  // Downloads
  var dl = createDownloadBar(data, {
    filenameBase: 'scope-export-' + fmtAddr(data.address),
    csvFn: function(d) {
      if (d.tokens && d.tokens.length > 0) {
        return arrayToCSV(
          ['symbol', 'name', 'balance', 'contract_address'],
          d.tokens.map(function(t) { return [t.symbol, t.name, t.formatted_balance || t.balance, t.contract_address]; })
        );
      }
      if (d.transactions && d.transactions.length > 0) {
        return arrayToCSV(
          ['hash', 'from', 'to', 'value', 'status'],
          d.transactions.map(function(tx) { return [tx.hash, tx.from, tx.to, tx.value, tx.status]; })
        );
      }
      return JSON.stringify(d);
    },
    mdFn: function(d) {
      var md = '# Data Export\n\n';
      md += '**Address:** `' + d.address + '`\n';
      md += '**Chain:** ' + d.chain + '\n\n';
      if (d.balance) {
        md += '## Balance\n\n- ' + (d.balance.formatted || d.balance.raw || '-') + '\n\n';
      }
      if (d.tokens && d.tokens.length > 0) {
        md += '## Tokens\n\n| Symbol | Balance | Contract |\n|--------|---------|----------|\n';
        d.tokens.forEach(function(t) {
          md += '| ' + t.symbol + ' | ' + (t.formatted_balance || t.balance) + ' | `' + t.contract_address + '` |\n';
        });
      }
      return md;
    }
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);
  resultEl.appendChild(view);
}

// ===== Insights Renderer =====
function renderInsights(resultEl, data) {
  // Insights wraps another type; detect and delegate
  if (data.target_info) {
    var type = data.target_info.type;
    var view = el('div', { className: 'result-view' });

    // Type badge
    var typeBadge = el('div', { className: 'result-header', style: { marginBottom: '12px' } });
    typeBadge.appendChild(el('span', { textContent: 'Detected:', style: { color: 'var(--text-muted)' } }));
    typeBadge.appendChild(chainBadge(data.target_info.chain));
    typeBadge.appendChild(el('span', {
      className: 'chain-badge',
      textContent: type,
      style: { background: 'rgba(188, 140, 255, 0.12)', color: 'var(--purple)' }
    }));
    view.appendChild(typeBadge);
    resultEl.appendChild(view);

    // Delegate to sub-renderer
    if (type === 'address' && data.data) {
      renderAddress(resultEl, data.data);
    } else if (type === 'transaction' && data.data) {
      renderTransaction(resultEl, data.data);
    } else if (type === 'token' && data.data) {
      renderCrawl(resultEl, data.data);
    } else {
      // Fallback for unknown insight types
      showFallbackResults(resultEl, data);
    }
  } else {
    // No target_info wrapper — might be direct data
    showFallbackResults(resultEl, data);
  }
}

// ===== Address Book Renderer =====
var _abData = []; // cached address book entries

function renderAddressBook(resultEl, data) {
  var entries = data.addresses || data.entries || [];
  if (!Array.isArray(entries)) {
    entries = [];
  }
  _abData = entries;

  clearElement(resultEl);
  var view = el('div', { className: 'result-view' });

  // Count
  view.appendChild(el('div', { className: 'ab-count' },
    entries.length + ' address' + (entries.length !== 1 ? 'es' : '') + ' saved'));

  if (entries.length === 0) {
    var wrap = el('div', { className: 'ab-table-wrap' });
    wrap.appendChild(el('div', { className: 'ab-empty' },
      'No saved addresses yet. Use the form above to add one.'));
    view.appendChild(wrap);
  } else {
    var wrap = el('div', { className: 'ab-table-wrap' });

    // Header row
    var hdr = el('div', { className: 'ab-row ab-row-header' });
    hdr.appendChild(el('span', null, 'Label'));
    hdr.appendChild(el('span', null, 'Address'));
    hdr.appendChild(el('span', null, 'Chain'));
    hdr.appendChild(el('span', null, 'Tags'));
    hdr.appendChild(el('span', null, ''));
    wrap.appendChild(hdr);

    entries.forEach(function(entry) {
      var row = el('div', { className: 'ab-row' });

      // Label
      var labelCell = el('div', {
        className: 'ab-label-cell' + (entry.label ? '' : ' no-label'),
        textContent: entry.label || '(no label)'
      });
      row.appendChild(labelCell);

      // Address (click to copy)
      var addrCell = el('div', {
        className: 'ab-addr-cell',
        textContent: entry.address,
        title: 'Click to copy',
        onClick: function() {
          navigator.clipboard.writeText(entry.address);
          addrCell.textContent = 'Copied!';
          setTimeout(function() { addrCell.textContent = entry.address; }, 1200);
        }
      });
      row.appendChild(addrCell);

      // Chain badge
      row.appendChild(chainBadge(entry.chain));

      // Tags
      var tagsCell = el('div', { className: 'ab-tags' });
      if (entry.tags && entry.tags.length > 0) {
        entry.tags.forEach(function(tag) {
          tagsCell.appendChild(el('span', { className: 'ab-tag', textContent: tag }));
        });
      }
      row.appendChild(tagsCell);

      // Actions
      var actions = el('div', { className: 'ab-actions' });

      // Analyze button
      actions.appendChild(el('button', {
        className: 'ab-btn-sm',
        textContent: 'Analyze',
        title: 'Open in Insights',
        onClick: function() {
          document.getElementById('insights-target').value = entry.address;
          var chainSel = document.getElementById('insights-chain');
          for (var i = 0; i < chainSel.options.length; i++) {
            if (chainSel.options[i].value === entry.chain) {
              chainSel.selectedIndex = i;
              break;
            }
          }
          // Switch to insights panel
          document.querySelectorAll('nav button').forEach(function(b) { b.classList.remove('active'); });
          document.querySelectorAll('.panel').forEach(function(p) { p.classList.remove('active'); });
          document.querySelector('[data-panel="insights"]').classList.add('active');
          document.getElementById('panel-insights').classList.add('active');
          runInsights();
        }
      }));

      // Delete button
      actions.appendChild(el('button', {
        className: 'ab-btn-sm danger',
        textContent: 'Remove',
        onClick: function() {
          removeAddressBookEntry(entry.address);
        }
      }));

      row.appendChild(actions);
      wrap.appendChild(row);
    });

    view.appendChild(wrap);
  }

  // Download bar
  var dl = createDownloadBar(entries, {
    filenameBase: 'scope-address-book',
    csvFn: function(d) {
      var arr = Array.isArray(d) ? d : [];
      return arrayToCSV(['label', 'chain', 'address', 'tags'],
        arr.map(function(e) { return [e.label || '', e.chain, e.address, (e.tags || []).join('; ')]; })
      );
    },
    mdFn: function(d) {
      var arr = Array.isArray(d) ? d : [];
      var md = '# Address Book\n\n';
      if (arr.length === 0) return md + 'No addresses saved.\n';
      md += '| Label | Chain | Address | Tags |\n|-------|-------|---------|------|\n';
      arr.forEach(function(e) {
        md += '| ' + (e.label || '-') + ' | ' + e.chain + ' | `' + e.address + '` | ' + (e.tags || []).join(', ') + ' |\n';
      });
      return md;
    }
  });
  view.appendChild(dl.bar);
  view.appendChild(dl.rawContainer);

  resultEl.appendChild(view);
}

// ===== Command Handlers =====

function runInsights() {
  var target = document.getElementById('insights-target').value.trim();
  if (!target) return;
  var chain = document.getElementById('insights-chain').value || undefined;
  apiPost('/api/insights', { target: target, chain: chain },
    document.getElementById('insights-results'), renderInsights);
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
  }, document.getElementById('address-results'), renderAddress);
}

function runTx() {
  var hash = document.getElementById('tx-hash').value.trim();
  if (!hash) return;
  apiPost('/api/tx', {
    hash: hash,
    chain: document.getElementById('tx-chain').value,
    decode: document.getElementById('tx-decode').checked,
    trace: document.getElementById('tx-trace').checked,
  }, document.getElementById('tx-results'), renderTransaction);
}

function runCrawl() {
  var token = document.getElementById('crawl-token').value.trim();
  if (!token) return;
  apiPost('/api/crawl', {
    token: token,
    chain: document.getElementById('crawl-chain').value,
    period: document.getElementById('crawl-period').value,
  }, document.getElementById('crawl-results'), renderCrawl);
}

function runDiscover() {
  var source = document.getElementById('disc-source').value;
  var chain = document.getElementById('disc-chain').value;
  var limit = document.getElementById('disc-limit').value;
  var url = '/api/discover?source=' + source + '&limit=' + limit;
  if (chain) url += '&chain=' + chain;
  apiGet(url, document.getElementById('discover-results'), renderDiscover);
}

function runTokenHealth() {
  var token = document.getElementById('th-token').value.trim();
  if (!token) return;
  apiPost('/api/token-health', {
    token: token,
    chain: document.getElementById('th-chain').value,
    with_market: document.getElementById('th-market').checked,
    market_venue: document.getElementById('th-venue').value,
  }, document.getElementById('th-results'), renderTokenHealth);
}

function runMarket() {
  var pair = document.getElementById('mkt-pair').value.trim();
  if (!pair) return;
  apiPost('/api/market/summary', {
    pair: pair,
    market_venue: document.getElementById('mkt-venue').value,
    peg: parseFloat(document.getElementById('mkt-peg').value) || 1.0,
  }, document.getElementById('market-results'), renderMarketSummary);
}

function runCompliance() {
  var address = document.getElementById('comp-address').value.trim();
  if (!address) return;
  apiPost('/api/compliance/risk', {
    address: address,
    chain: document.getElementById('comp-chain').value,
    detailed: document.getElementById('comp-detailed').checked,
  }, document.getElementById('compliance-results'), renderCompliance);
}

function runExport() {
  var address = document.getElementById('exp-address').value.trim();
  if (!address) return;
  apiPost('/api/export', {
    address: address,
    chain: document.getElementById('exp-chain').value,
    format: 'json',
  }, document.getElementById('export-results'), renderExport);
}

// ===== Address Book =====
function loadAddressBook() {
  apiGet('/api/address-book/list', document.getElementById('ab-table'), function(resultEl, data) {
    renderAddressBook(resultEl, data);
    updateAddressBookSuggestions(data);
  });
}

function updateAddressBookSuggestions(data) {
  var entries = data.addresses || data.entries || [];
  if (!Array.isArray(entries)) entries = [];
  var datalist = document.getElementById('ab-suggestions');
  if (!datalist) return;
  clearElement(datalist);
  entries.forEach(function(entry) {
    if (entry.label) {
      var opt = document.createElement('option');
      opt.value = '@' + entry.label;
      opt.label = entry.label + ' (' + entry.chain + ': ' + fmtAddr(entry.address) + ')';
      datalist.appendChild(opt);
    }
    // Also suggest the raw address
    var addrOpt = document.createElement('option');
    addrOpt.value = entry.address;
    addrOpt.label = (entry.label || fmtAddr(entry.address)) + ' (' + entry.chain + ')';
    datalist.appendChild(addrOpt);
  });
}

function addAddressBookEntry() {
  var address = document.getElementById('ab-address').value.trim();
  if (!address) return;

  var tagsRaw = document.getElementById('ab-tags').value.trim();
  var tags = tagsRaw ? tagsRaw.split(',').map(function(t) { return t.trim(); }).filter(Boolean) : [];

  var resultEl = document.getElementById('ab-table');
  showLoading(resultEl);

  fetch(API + '/api/address-book/add', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      address: address,
      chain: document.getElementById('ab-chain').value,
      label: document.getElementById('ab-label').value.trim() || undefined,
      tags: tags,
    }),
  }).then(function(res) { return res.json(); })
    .then(function(data) {
      if (data.error) {
        showError(resultEl, 'Error: ' + data.error);
        return;
      }
      // Clear the form
      document.getElementById('ab-address').value = '';
      document.getElementById('ab-label').value = '';
      document.getElementById('ab-tags').value = '';
      // Render the updated list and refresh suggestions
      if (data.addresses) {
        clearElement(resultEl);
        renderAddressBook(resultEl, data);
        updateAddressBookSuggestions(data);
      } else {
        loadAddressBook();
      }
    })
    .catch(function(e) {
      showError(resultEl, 'Request failed: ' + e.message);
    });
}

function removeAddressBookEntry(address) {
  var resultEl = document.getElementById('ab-table');
  showLoading(resultEl);

  fetch(API + '/api/address-book/remove', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ address: address }),
  }).then(function(res) { return res.json(); })
    .then(function(data) {
      if (data.error) {
        showError(resultEl, 'Error: ' + data.error);
        return;
      }
      if (data.addresses) {
        clearElement(resultEl);
        renderAddressBook(resultEl, data);
        updateAddressBookSuggestions(data);
      } else {
        loadAddressBook();
      }
    })
    .catch(function(e) {
      showError(resultEl, 'Request failed: ' + e.message);
    });
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
    var hasNone = sel.options.length > 0 && sel.options[0].value === '';
    var currentValue = sel.value;
    while (sel.options.length > (hasNone ? 1 : 0)) {
      sel.remove(hasNone ? 1 : 0);
    }
    venues.forEach(function(v) {
      var opt = document.createElement('option');
      opt.value = v.id;
      opt.textContent = v.name;
      sel.appendChild(opt);
    });
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
      clearElement(resultEl);
      renderExchangeSnapshot(resultEl, data);
    })
    .catch(function(e) {
      showError(resultEl, 'Request failed: ' + e.message);
    });
}

function renderExchangeSnapshot(parentEl, data) {
  var view = el('div', { className: 'result-view' });

  var grid = document.createElement('div');
  grid.className = 'exchange-grid';

  var leftCol = document.createElement('div');
  leftCol.className = 'exchange-col';

  if (data.ticker) {
    var tickerH3 = document.createElement('h3');
    tickerH3.textContent = 'Ticker \u2014 ' + (data.pair || '');
    leftCol.appendChild(tickerH3);
    renderTicker(leftCol, data.ticker);
  }

  if (data.order_book) {
    var obH3 = document.createElement('h3');
    obH3.style.marginTop = '12px';
    obH3.textContent = 'Order Book';
    leftCol.appendChild(obH3);
    renderOrderBook(leftCol, data.order_book);
  }

  grid.appendChild(leftCol);

  var rightCol = document.createElement('div');
  rightCol.className = 'exchange-col';
  var trH3 = document.createElement('h3');
  trH3.textContent = 'Recent Trades';
  rightCol.appendChild(trH3);

  if (data.recent_trades && data.recent_trades.length > 0) {
    renderTradeHistory(rightCol, data.recent_trades);
  } else {
    rightCol.appendChild(el('div', {
      textContent: 'No trade data available for this venue.',
      style: { color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: '12px' }
    }));
  }

  grid.appendChild(rightCol);
  view.appendChild(grid);

  // Downloads
  var dlOpts = {
    filenameBase: 'scope-exchange-' + (data.venue || '') + '-' + (data.pair || ''),
    csvFn: function(d) {
      if (d.recent_trades && d.recent_trades.length > 0) {
        return arrayToCSV(
          ['side', 'price', 'quantity', 'timestamp'],
          d.recent_trades.map(function(t) { return [t.side, t.price, t.quantity, t.timestamp_ms]; })
        );
      }
      return JSON.stringify(d);
    },
    mdFn: function(d) {
      var md = '# Exchange Snapshot: ' + (d.venue || '') + ' ' + (d.pair || '') + '\n\n';
      if (d.ticker) {
        md += '## Ticker\n\n';
        md += '| Metric | Value |\n|--------|-------|\n';
        md += '| Last Price | ' + formatPrice(d.ticker.last_price) + ' |\n';
        md += '| 24h High | ' + formatPrice(d.ticker.high_24h) + ' |\n';
        md += '| 24h Low | ' + formatPrice(d.ticker.low_24h) + ' |\n';
        md += '| Volume | ' + fmtCompact(d.ticker.volume_24h) + ' |\n';
      }
      return md;
    }
  };
  var dlResult = createDownloadBar(data, dlOpts);
  view.appendChild(dlResult.bar);
  view.appendChild(dlResult.rawContainer);

  parentEl.appendChild(view);
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
    row.appendChild(el('span', { className: 'ticker-label', textContent: f[0] }));
    row.appendChild(el('span', { className: 'ticker-value', textContent: f[1] }));
    parent.appendChild(row);
  });
}

function renderOrderBook(parent, ob) {
  var table = document.createElement('table');
  table.className = 'ob-table';

  var thead = document.createElement('thead');
  var hr = document.createElement('tr');
  ['Price', 'Quantity', 'Value'].forEach(function(h) {
    hr.appendChild(el('th', { textContent: h }));
  });
  thead.appendChild(hr);
  table.appendChild(thead);

  var tbody = document.createElement('tbody');

  var asks = (ob.asks || []).slice(0, 10).reverse();
  asks.forEach(function(level) {
    var tr = el('tr', { className: 'ob-ask' });
    addOBCell(tr, formatPrice(level.price));
    addOBCell(tr, fmtQty(level.quantity));
    addOBCell(tr, fmtCompact(level.value));
    tbody.appendChild(tr);
  });

  if (ob.spread !== null && ob.spread !== undefined) {
    var sr = el('tr', { className: 'ob-spread' });
    var sd = el('td');
    sd.colSpan = 3;
    sd.textContent = 'Spread: ' + formatPrice(ob.spread) +
      (ob.mid_price ? ' | Mid: ' + formatPrice(ob.mid_price) : '');
    sr.appendChild(sd);
    tbody.appendChild(sr);
  }

  var bids = (ob.bids || []).slice(0, 10);
  bids.forEach(function(level) {
    var tr = el('tr', { className: 'ob-bid' });
    addOBCell(tr, formatPrice(level.price));
    addOBCell(tr, fmtQty(level.quantity));
    addOBCell(tr, fmtCompact(level.value));
    tbody.appendChild(tr);
  });

  table.appendChild(tbody);
  parent.appendChild(table);
}

function addOBCell(row, text) {
  row.appendChild(el('td', { textContent: text }));
}

function renderTradeHistory(parent, trades) {
  var hdr = el('div', { className: 'trade-row', style: { fontWeight: '600', color: 'var(--text-muted)', fontSize: '10px', textTransform: 'uppercase' } });
  ['Side', 'Price', 'Qty', 'Time'].forEach(function(h) {
    hdr.appendChild(el('span', {
      className: h === 'Side' ? 'trade-side' : h === 'Price' ? 'trade-price' : h === 'Qty' ? 'trade-qty' : 'trade-time',
      textContent: h
    }));
  });
  parent.appendChild(hdr);

  trades.slice(0, 50).forEach(function(t) {
    var row = el('div', { className: 'trade-row ' + (t.side === 'buy' ? 'trade-buy' : 'trade-sell') });
    row.appendChild(el('span', { className: 'trade-side', textContent: t.side === 'buy' ? 'B' : 'S' }));
    row.appendChild(el('span', { className: 'trade-price', textContent: formatPrice(t.price) }));
    row.appendChild(el('span', { className: 'trade-qty', textContent: fmtQty(t.quantity) }));
    row.appendChild(el('span', { className: 'trade-time', textContent: formatTradeTime(t.timestamp_ms) }));
    parent.appendChild(row);
  });
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
      card.appendChild(el('div', { className: 'stat-value error', textContent: data.message }));
      statsEl.appendChild(card);
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
  priceHistory.push(data.price_usd);
  if (priceHistory.length > 60) priceHistory.shift();

  var canvas = document.getElementById('price-canvas');
  var ctx = canvas.getContext('2d');
  var w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);

  if (priceHistory.length > 1) {
    var min = Math.min.apply(null, priceHistory) * 0.999;
    var max = Math.max.apply(null, priceHistory) * 1.001;
    var range = max - min || 1;
    var pad = 30;

    ctx.strokeStyle = '#1e2530';
    ctx.lineWidth = 0.5;
    ctx.fillStyle = '#6e7a8a';
    ctx.font = '10px monospace';
    ctx.textAlign = 'right';
    for (var i = 0; i <= 4; i++) {
      var gy = pad + ((h - pad * 2) / 4) * i;
      ctx.beginPath(); ctx.moveTo(pad, gy); ctx.lineTo(w, gy); ctx.stroke();
      var priceAtLine = max - (range * i / 4);
      ctx.fillText('$' + priceAtLine.toFixed(4), pad - 4, gy + 3);
    }

    var gradient = ctx.createLinearGradient(0, 0, 0, h);
    gradient.addColorStop(0, 'rgba(63, 185, 80, 0.15)');
    gradient.addColorStop(1, 'rgba(63, 185, 80, 0.0)');

    var points = [];
    priceHistory.forEach(function(p, idx) {
      var x = pad + (idx / (priceHistory.length - 1)) * (w - pad);
      var y = pad + (h - pad * 2) - ((p - min) / range) * (h - pad * 2);
      points.push([x, y]);
    });

    ctx.beginPath();
    ctx.moveTo(points[0][0], points[0][1]);
    for (var j = 1; j < points.length; j++) {
      ctx.lineTo(points[j][0], points[j][1]);
    }
    ctx.lineTo(points[points.length - 1][0], h);
    ctx.lineTo(points[0][0], h);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    ctx.shadowColor = 'rgba(63, 185, 80, 0.4)';
    ctx.shadowBlur = 8;
    ctx.strokeStyle = '#3fb950';
    ctx.lineWidth = 2;
    ctx.beginPath();
    points.forEach(function(pt, idx) {
      if (idx === 0) ctx.moveTo(pt[0], pt[1]); else ctx.lineTo(pt[0], pt[1]);
    });
    ctx.stroke();

    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;

    var lastPt = points[points.length - 1];
    ctx.beginPath();
    ctx.arc(lastPt[0], lastPt[1], 4, 0, Math.PI * 2);
    ctx.fillStyle = '#f85149';
    ctx.fill();
    ctx.beginPath();
    ctx.arc(lastPt[0], lastPt[1], 7, 0, Math.PI * 2);
    ctx.strokeStyle = 'rgba(248, 81, 73, 0.3)';
    ctx.lineWidth = 2;
    ctx.stroke();

    ctx.fillStyle = '#f0f6fc';
    ctx.font = 'bold 15px monospace';
    ctx.textAlign = 'right';
    ctx.fillText('$' + data.price_usd.toFixed(6), w - 8, 20);
  }

  var statsEl = document.getElementById('mon-stats');
  clearElement(statsEl);

  var change24 = data.price_change_24h || 0;
  var changeClass = change24 >= 0 ? 'positive' : 'negative';
  var changeSign = change24 >= 0 ? '+' : '';

  function addStat(label, value, extra) {
    var card = document.createElement('div');
    card.className = 'stat-card';
    card.appendChild(el('div', { className: 'stat-label', textContent: label }));
    var val = el('div', { className: 'stat-value' });
    if (typeof value === 'object' && value !== null) {
      val.appendChild(value);
    } else {
      val.textContent = value;
    }
    card.appendChild(val);
    if (extra) {
      card.appendChild(el('div', { className: 'stat-change ' + extra.cls, textContent: extra.text }));
    }
    statsEl.appendChild(card);
  }

  addStat((data.token ? data.token.symbol : '') + ' Price',
    '$' + (data.price_usd ? data.price_usd.toFixed(6) : '-'),
    { cls: changeClass, text: changeSign + change24.toFixed(2) + '% (24h)' });
  addStat('Volume (24h)', fmtCompact(data.volume_24h));
  addStat('Liquidity', fmtCompact(data.liquidity_usd));
  addStat('Market Cap', fmtCompact(data.market_cap));

  var bsSpan = document.createElement('span');
  bsSpan.style.fontSize = '16px';
  var buySpan = el('span', { style: { color: 'var(--green)' }, textContent: String(data.buys_24h || 0) });
  var sellSpan = el('span', { style: { color: 'var(--red)' }, textContent: String(data.sells_24h || 0) });
  bsSpan.appendChild(buySpan);
  bsSpan.appendChild(document.createTextNode(' / '));
  bsSpan.appendChild(sellSpan);
  addStat('Buys / Sells (24h)', bsSpan);
  addStat('Last Update', data.timestamp || '-');

  if (data.exchange_ticker) {
    var t = data.exchange_ticker;
    addStat('CEX Price', formatPrice(t.last_price));
    if (t.volume_24h) addStat('CEX Volume', fmtCompact(t.volume_24h));
  }

  if (data.exchange_order_book || data.exchange_trades) {
    updateMonitorExchange(data);
  }
}

function updateMonitorExchange(data) {
  var panel = document.getElementById('mon-exchange');
  if (!panel) return;
  panel.style.display = 'grid';

  var obEl = document.getElementById('mon-orderbook');
  clearElement(obEl);
  if (data.exchange_order_book) {
    obEl.appendChild(el('h4', { textContent: 'Order Book' }));
    renderOrderBook(obEl, data.exchange_order_book);
  }

  var trEl = document.getElementById('mon-trades');
  clearElement(trEl);
  if (data.exchange_trades && data.exchange_trades.length > 0) {
    trEl.appendChild(el('h4', { textContent: 'Recent Trades' }));
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

    var fileInfo = el('div', { style: { marginBottom: '12px' } });
    fileInfo.appendChild(el('strong', null, 'Config file: '));
    fileInfo.appendChild(el('code', { textContent: data.config_path || 'unknown' }));
    fileInfo.appendChild(document.createTextNode(' '));
    fileInfo.appendChild(el('span', {
      className: data.config_exists ? 'badge badge-set' : 'badge badge-unset',
      textContent: data.config_exists ? 'exists' : 'not found'
    }));
    statusEl.appendChild(fileInfo);

    var keys = data.api_keys || {};
    statusEl.appendChild(el('h3', {
      textContent: 'API Keys',
      style: { color: 'var(--text-bright)', marginBottom: '8px' }
    }));

    var keysGrid = el('div', { className: 'setup-grid' });
    Object.entries(keys).forEach(function(entry) {
      var card = el('div', { className: 'setup-card' });
      card.appendChild(el('h3', { textContent: entry[0] }));
      card.appendChild(el('span', {
        className: entry[1] ? 'badge badge-set' : 'badge badge-unset',
        textContent: entry[1] ? 'Configured' : 'Not set'
      }));
      keysGrid.appendChild(card);
    });
    statusEl.appendChild(keysGrid);

    var rpcs = data.rpc_endpoints || {};
    statusEl.appendChild(el('h3', {
      textContent: 'RPC Endpoints',
      style: { color: 'var(--text-bright)', margin: '12px 0 8px' }
    }));

    var rpcsGrid = el('div', { className: 'setup-grid' });
    Object.entries(rpcs).forEach(function(entry) {
      var card = el('div', { className: 'setup-card' });
      card.appendChild(el('h3', { textContent: entry[0] }));
      card.appendChild(el('span', {
        className: entry[1] ? 'badge badge-set' : 'badge badge-unset',
        textContent: entry[1] ? 'Configured' : 'Not set'
      }));
      rpcsGrid.appendChild(card);
    });
    statusEl.appendChild(rpcsGrid);
  } catch (e) {
    var statusEl2 = document.getElementById('setup-status');
    clearElement(statusEl2);
    statusEl2.appendChild(el('div', { className: 'error', textContent: 'Failed to load config status' }));
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
