'use strict';

(() => {
  const $ = (selector, root = document) => root.querySelector(selector);
  const nonce = $('meta[name="sdk-demo-nonce"]').content;
  const ui = {
    cards: new Map(), jobs: new Map(), ready: false, connected: false,
    credential: { ready: false }, busy: new Set(),
    lastUpdate: 0, lastStates: new Map(), pollTimer: null, toastTimer: null,
  };
  const GROUPS = [
    { id: 'web', title: 'Web & scripting', number: '01', languages: ['typescript', 'python', 'ruby', 'php'] },
    { id: 'systems', title: 'Compiled & cross-platform', number: '02', languages: ['go', 'rust', 'cpp', 'dart'] },
    { id: 'platform', title: 'Platform & JVM', number: '03', languages: ['swift', 'kotlin', 'java', 'csharp'] },
  ];
  const stateNames = { queued: 'Queued', running: 'Running', completed: 'Confirmed', failed: 'Failed', cancelled: 'Cancelled' };
  const failureMessages = {
    'deadline-exceeded': 'The native program reached its 22-second process deadline and was stopped.',
    'output-truncated': 'A native output stream exceeded the capture limit. That entire stream was omitted; this run cannot count as a success.',
    'runtime-pin-mismatch': 'The prepared source, package, or executable no longer matches its accepted pin. Check the source guide and restart after restoring the prepared runtime.',
    'launch-failed': 'The prepared native executable could not be started. Check the local runtime described in the source guide.',
    'invalid-or-missing-native-confirmation': 'The native program did not return one valid, decoded confirmation record.',
    'capture-failed': 'The local server could not finish capturing the native program’s confirmation.',
    'cancelled': 'This job was cancelled. Its owned process group has been stopped.',
    'execution-failed': 'The local execution could not finish. Check the prepared runtime and try again.',
  };
  const playIcon = '<svg width="11" height="11" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M4 2.8a.7.7 0 0 1 1.1-.6l8 5.2a.7.7 0 0 1 0 1.2l-8 5.2a.7.7 0 0 1-1.1-.6z"/></svg>';
  const stopIcon = '<svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true"><rect x="2" y="2" width="8" height="8" rx="1"/></svg>';

  function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
  }

  function highlight(line) {
    const keywords = new Set(['import', 'from', 'const', 'let', 'var', 'val', 'final', 'with', 'as', 'await', 'try', 'using', 'auto', 'if', 'return', 'defer', 'use', 'do', 'end', 'echo']);
    const types = new Set(['Client', 'IoTransport', 'GetCurrentKeyStatus200', 'GetCurrentKeyInput', 'OpenRouter', 'Sdk']);
    const pattern = /("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b[A-Za-z_][\w]*\b|\b\d[\d_]*\b)/g;
    let output = '';
    let end = 0;
    for (const match of line.matchAll(pattern)) {
      const token = match[0];
      output += escapeHtml(line.slice(end, match.index));
      let kind = '';
      if (/^["']/.test(token)) kind = 'string';
      else if (keywords.has(token)) kind = 'keyword';
      else if (types.has(token)) kind = 'type';
      else if (/^\d/.test(token) || ['true', 'false', 'null', 'nil'].includes(token)) kind = 'number';
      else if (/^\s*[(!]/.test(line.slice(match.index + token.length))) kind = 'function';
      output += kind ? `<span class="syntax-${kind}">${escapeHtml(token)}</span>` : escapeHtml(token);
      end = match.index + token.length;
    }
    return output + escapeHtml(line.slice(end));
  }

  function setText(element, value) {
    if (element.textContent !== String(value)) element.textContent = value;
  }

  function toast(message) {
    clearTimeout(ui.toastTimer);
    setText($('#toast'), message);
    $('#toast').hidden = false;
    ui.toastTimer = setTimeout(() => { $('#toast').hidden = true; }, 2600);
  }

  async function api(path, body) {
    const controller = new AbortController();
    const deadline = setTimeout(() => controller.abort(), 5000);
    try {
      const response = await fetch(path, {
        method: body === undefined ? 'GET' : 'POST',
        headers: { 'X-SDK-Demo-Nonce': nonce, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }) },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        mode: 'same-origin', credentials: 'omit', cache: 'no-store', signal: controller.signal,
      });
      const value = await response.json();
      if (!response.ok) {
        const error = new Error(value.message || 'The local request could not finish.');
        error.code = value.error;
        throw error;
      }
      return value;
    } catch (error) {
      if (!error.code) {
        const safe = new Error('The local server is unavailable. Start ./demo-web.sh, then reconnect.');
        safe.code = 'connection-unavailable';
        throw safe;
      }
      throw error;
    } finally {
      clearTimeout(deadline);
    }
  }

  function showConnectionError(message) {
    setText($('#connection-message'), message);
    $('#connection-error').hidden = false;
  }

  function keyError(message) {
    const element = $('#key-error');
    element.hidden = !message;
    setText(element, message || '');
    $('#api-key').setAttribute('aria-invalid', message ? 'true' : 'false');
  }

  function requireKey(language) {
    if (ui.credential.ready) return true;
    keyError(`Add your OpenRouter API key to ${language === 'all' ? 'run the collection' : `run ${ui.cards.get(language)?.data.name || 'this SDK'}`}.`);
    $('#api-key').focus({ preventScroll: true });
    $('#credential-form').scrollIntoView({ behavior: 'smooth', block: 'center' });
    return false;
  }

  function updateCredential(credential) {
    ui.credential = credential;
    const label = credential.ready ? 'Key loaded in memory' : 'No key loaded';
    setText($('#credential-state'), label);
    $('#credential-state').classList.toggle('loaded', credential.ready);
    $('#clear-key').disabled = !credential.ready || ui.busy.has('credential');
    setText($('#save-key'), credential.ready ? 'Replace key' : 'Use key');
  }

  function cardMarkup(card) {
    const esc = escapeHtml;
    const lines = card.code.split('\n').map((line, index) => `<span class="code-line"><span class="line-number" aria-hidden="true">${index + 1}</span><span class="line-content">${highlight(line)}</span></span>`).join('');
    return `
      <div class="card-header">
        <span class="language-badge ${esc(card.id)}" aria-hidden="true">${esc(card.badge)}</span>
        <div class="language-heading"><h4 id="title-${esc(card.id)}">${esc(card.name)}</h4><code class="package-identity">${esc(card.identity)}</code></div>
        <span class="card-state">${card.ready ? 'Ready' : 'Unavailable'}</span>
      </div>
      <div class="code-window">
        <div class="code-toolbar"><span class="code-filename">${esc(card.file)}</span><button class="copy-code" type="button" aria-label="Copy ${esc(card.name)} example"><svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><rect x="5" y="5" width="8" height="9" rx="1.4"/><path d="M3 11H2.5A1.5 1.5 0 0 1 1 9.5v-7A1.5 1.5 0 0 1 2.5 1h6A1.5 1.5 0 0 1 10 2.5V3"/></svg><span>Copy</span></button></div>
        <pre class="code-block" tabindex="0" aria-label="Four-line ${esc(card.name)} SDK example"><code>${lines}</code></pre>
      </div>
      <div class="card-context"><div class="card-flavor"><span>${esc(card.flavor)}</span><span class="package-version">v${esc(card.version)} · local</span></div><p class="assumption"><span class="assumption-label">Context</span>${esc(card.assumption)}</p></div>
      <div class="card-footer"><div class="run-line"><button class="button run-button" type="button" aria-label="Run ${esc(card.name)} live request">${playIcon}<span>Run live request</span></button><div class="run-status"><span class="status-caption">${card.ready ? 'Not run yet' : 'Preflight unavailable'}</span><span class="elapsed">${card.ready ? 'Native SDK ready' : 'See source guide'}</span></div></div><div class="card-links"><a href="${esc(card.sourceUrl)}" target="_blank" rel="noopener noreferrer">Full program ↗</a><a href="${esc(card.docsUrl)}" target="_blank" rel="noopener noreferrer">Package README ↗</a><span class="local-package">${card.native ? 'NATIVE CONSUMER' : 'JAVASCRIPT BONUS'}</span></div></div>
      <details class="result-details" hidden><summary>View native result</summary><p class="result-message"></p><dl class="result-meta"><div><dt>HTTP status</dt><dd class="result-http"></dd></div><div><dt>Native outcome</dt><dd class="result-kind"></dd></div></dl><pre class="result-json" tabindex="0" aria-label="${esc(card.name)} decoded confirmation"></pre></details>`;
  }

  function makeCard(card) {
    const article = document.createElement('article');
    article.id = `sdk-${card.id}`;
    article.className = 'sdk-card';
    article.dataset.language = card.id;
    article.dataset.native = String(card.native);
    article.dataset.state = card.ready ? 'ready' : 'unavailable';
    article.setAttribute('aria-labelledby', `title-${card.id}`);
    article.innerHTML = cardMarkup(card);
    const entry = { data: card, element: article, lastDetail: null };
    ui.cards.set(card.id, entry);
    $('.copy-code', article).addEventListener('click', () => copyCode(entry));
    $('.run-button', article).addEventListener('click', () => {
      const job = ui.jobs.get(card.id);
      if (job && ['running', 'queued'].includes(job.state)) cancelJob(job);
      else start(card.id);
    });
    return article;
  }

  function renderCollection(cards) {
    const data = new Map(cards.map(card => [card.id, card]));
    const root = $('#sdk-groups');
    root.replaceChildren();
    $('#language-index').replaceChildren();
    for (const group of GROUPS) {
      const section = document.createElement('section');
      section.className = 'sdk-group';
      section.setAttribute('aria-labelledby', `group-${group.id}`);
      section.innerHTML = `<div class="group-heading"><span>${group.number}</span><h3 id="group-${group.id}">${group.title}</h3></div><div class="sdk-grid"></div>`;
      for (const language of group.languages) {
        const card = data.get(language);
        $('.sdk-grid', section).append(makeCard(card));
        const link = document.createElement('a');
        link.href = `#sdk-${language}`;
        link.dataset.language = language;
        link.innerHTML = `<span class="index-dot" aria-hidden="true"></span>${escapeHtml(card.name)}`;
        $('#language-index').append(link);
      }
      root.append(section);
    }
    $('#bonus-sdk').replaceChildren(makeCard(data.get('javascript')));
    root.setAttribute('aria-busy', 'false');
    const prepared = cards.filter(card => card.native && card.ready).length;
    setText($('#ready-count'), prepared);
    setText($('#preflight-label'), `${prepared}/12 native SDKs prepared`);
  }

  async function copyCode(entry) {
    const button = $('.copy-code', entry.element);
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(entry.data.code);
      } else {
        const field = document.createElement('textarea');
        field.value = entry.data.code;
        field.className = 'sr-only';
        document.body.append(field);
        field.select();
        const copied = document.execCommand('copy');
        field.remove();
        button.focus({ preventScroll: true });
        if (!copied) throw new Error('Copy unavailable');
      }
      setText($('span', button), 'Copied');
      toast(`${entry.data.name} example copied.`);
      setTimeout(() => setText($('span', button), 'Copy'), 2000);
    } catch {
      toast('Clipboard unavailable. Select the example to copy it.');
    }
  }

  function describeResult(job) {
    const result = job.result;
    if (job.state === 'completed') return 'HTTP 200 confirmed by the native SDK. The response was decoded successfully; usage is preserved as an exact number token.';
    if (result?.httpStatus === 401) return 'OpenRouter returned HTTP 401. The native SDK surfaced the API error. Check the key, replace it above, and run again.';
    if (result?.httpStatus === 403) return 'OpenRouter returned HTTP 403. Check this key’s permissions, then run again.';
    if (result?.httpStatus === 429) return 'OpenRouter returned HTTP 429. Wait before starting another live request.';
    if (failureMessages[result?.kind]) return failureMessages[result.kind];
    if (result?.httpStatus) return `The native SDK reported HTTP ${result.httpStatus}. The run did not produce a successful key confirmation.`;
    return 'The native SDK did not confirm a successful response. Available typed error metadata is shown below; no HTTP status has been invented.';
  }

  function updateCard(entry, job) {
    const root = entry.element;
    const active = job && ['queued', 'running'].includes(job.state);
    const busy = ui.busy.has(entry.data.id);
    const state = job?.state || (entry.data.ready ? 'ready' : 'unavailable');
    root.dataset.state = state;
    root.setAttribute('aria-busy', active ? 'true' : 'false');
    setText($('.card-state', root), job?.cancelRequested && active ? 'Stopping' : stateNames[state] || (entry.data.ready ? 'Ready' : 'Unavailable'));
    const button = $('.run-button', root);
    const mode = active ? 'cancel' : 'run';
    if (button.dataset.mode !== mode) {
      button.innerHTML = `${active ? stopIcon : playIcon}<span>${active ? 'Cancel request' : 'Run live request'}</span>`;
      button.dataset.mode = mode;
    }
    button.classList.toggle('cancel-button', Boolean(active));
    button.setAttribute('aria-label', `${active ? 'Cancel' : 'Run'} ${entry.data.name} ${active ? 'request' : 'live request'}`);
    button.disabled = !entry.data.ready || !ui.connected || busy || Boolean(job?.cancelRequested && active);

    const caption = $('.status-caption', root);
    let captionText = 'Not run yet';
    if (!entry.data.ready) captionText = 'Preflight unavailable';
    else if (job?.cancelRequested && active) captionText = 'Stopping native job…';
    else if (state === 'running') captionText = 'Request in progress';
    else if (state === 'queued') captionText = `Queue position ${job.queuePosition || '…'}`;
    else if (state === 'completed') captionText = 'HTTP 200 · Decoded';
    else if (state === 'failed') captionText = job.result?.httpStatus ? `HTTP ${job.result.httpStatus} · Failed` : (job.result?.kind === 'deadline-exceeded' ? 'Deadline exceeded' : 'Request failed');
    else if (state === 'cancelled') captionText = 'Request cancelled';
    if (caption.dataset.message !== captionText) {
      caption.replaceChildren();
      if (state === 'running') {
        const spinner = document.createElement('span');
        spinner.className = 'spinner';
        spinner.setAttribute('aria-hidden', 'true');
        caption.append(spinner);
      }
      caption.append(document.createTextNode(captionText));
      caption.dataset.message = captionText;
    }
    const elapsed = $('.elapsed', root);
    let timing = entry.data.ready ? 'Native SDK ready' : 'See source guide';
    if (job) timing = state === 'queued' ? 'Waiting for a worker' : `${(job.elapsedMs / 1000).toFixed(1)}s${state === 'running' ? ' elapsed' : ' · native process'}`;
    setText(elapsed, timing);

    const details = $('.result-details', root);
    details.hidden = !job?.result;
    if (job?.result && entry.lastDetail !== job.id) {
      const result = job.result;
      setText($('.result-message', root), describeResult(job));
      setText($('.result-http', root), result.httpStatus ?? 'Not received');
      setText($('.result-kind', root), result.kind);
      const record = result.confirmation || { ok: false, kind: result.kind, ...(Object.values(result.outputTruncated).some(Boolean) ? { outputTruncated: result.outputTruncated } : {}) };
      setText($('.result-json', root), JSON.stringify(record, null, 2));
      details.open = state === 'failed';
      entry.lastDetail = job.id;
    }
    const index = $(`.language-index a[data-language="${entry.data.id}"]`);
    if (index) index.dataset.state = state;
  }

  function updateButtons() {
    const active = [...ui.jobs.values()].some(job => ['queued', 'running'].includes(job.state));
    const allActive = [...ui.cards.values()].filter(entry => entry.data.native).every(entry => {
      const job = ui.jobs.get(entry.data.id);
      return job && ['queued', 'running'].includes(job.state);
    });
    $('#run-all').disabled = !ui.ready || !ui.connected || ui.busy.has('all') || allActive;
    $('#cancel-all').disabled = !active || ui.busy.has('cancel-all');
    for (const entry of ui.cards.values()) updateCard(entry, ui.jobs.get(entry.data.id));
  }

  function applySnapshot(snapshot) {
    if (snapshot.serverTime < ui.lastUpdate) return;
    ui.lastUpdate = snapshot.serverTime;
    ui.connected = !snapshot.closing;
    ui.jobs = new Map(snapshot.jobs.map(job => [job.language, job]));
    updateCredential(snapshot.credential);
    for (const key of ['running', 'queued', 'failed', 'cancelled']) setText($(`#${key}-count`), snapshot.counts[key]);
    setText($('#confirmed-count'), snapshot.counts.completed);
    $('#cancelled-stat').hidden = !snapshot.counts.cancelled;
    const active = snapshot.counts.running + snapshot.counts.queued;
    setText($('#session-caption'), active ? 'Native jobs run in the background. Keep exploring.' : snapshot.submitted ? 'Session results · Ready to run another request.' : 'Ready is preflight. Run to confirm live.');
    if (snapshot.receiptError) showConnectionError('A local receipt could not be written. The native result is still shown in this session.');
    else $('#connection-error').hidden = ui.connected;
    const changes = [];
    for (const job of snapshot.jobs) {
      const key = `${job.id}:${job.state}`;
      if (ui.lastStates.get(job.language) !== key) {
        ui.lastStates.set(job.language, key);
        changes.push(`${ui.cards.get(job.language)?.data.name || job.language}: ${stateNames[job.state]}.`);
      }
    }
    if (changes.length) setText($('#activity-announcement'), changes.join(' '));
    updateButtons();
  }

  async function refresh() {
    const snapshot = await api('/api/jobs');
    applySnapshot(snapshot);
    return snapshot;
  }

  async function poll() {
    clearTimeout(ui.pollTimer);
    let delay = 1600;
    try {
      const snapshot = await refresh();
      if (snapshot.jobs.some(job => ['running', 'queued'].includes(job.state))) delay = 220;
    } catch (error) {
      ui.connected = false;
      showConnectionError(error.message);
      updateButtons();
      delay = 2500;
    }
    if (document.hidden) delay = Math.max(delay, 2000);
    ui.pollTimer = setTimeout(poll, delay);
  }

  async function start(language) {
    if (ui.busy.has(language) || !requireKey(language)) return;
    ui.busy.add(language);
    updateButtons();
    try {
      const response = await api('/api/jobs', { language, operation: 'key' });
      toast(response.accepted ? `${response.accepted === 1 ? 'Native request' : `${response.accepted} native requests`} queued.` : 'Those native requests are already active.');
      await refresh();
      clearTimeout(ui.pollTimer);
      ui.pollTimer = setTimeout(poll, 100);
    } catch (error) {
      if (error.code === 'key-required') {
        updateCredential({ ready: false });
        requireKey(language);
      } else showConnectionError(error.message);
    } finally {
      ui.busy.delete(language);
      updateButtons();
    }
  }

  async function cancelJob(job) {
    if (ui.busy.has(job.language)) return;
    ui.busy.add(job.language);
    updateButtons();
    try {
      await api('/api/cancel', { id: job.id });
      await refresh();
    } catch (error) {
      showConnectionError(error.message);
    } finally {
      ui.busy.delete(job.language);
      updateButtons();
    }
  }

  $('#credential-form').addEventListener('submit', async event => {
    event.preventDefault();
    if (ui.busy.has('credential')) return;
    const input = $('#api-key');
    if (!input.value) {
      keyError('Enter your OpenRouter API key. Saving it does not start a request.');
      input.focus();
      return;
    }
    ui.busy.add('credential');
    $('#save-key').disabled = true;
    $('#clear-key').disabled = true;
    keyError('');
    // The password is sent only in this POST body, then removed from the field.
    // It is never retained in application state, a URL, or browser storage.
    const pending = api('/api/credential', { token: input.value });
    input.value = '';
    try {
      const response = await pending;
      updateCredential(response.credential);
      toast('Key loaded. Choose a language or run all 12.');
      await refresh();
      $('#run-all').focus({ preventScroll: true });
    } catch (error) {
      keyError(error.message);
    } finally {
      ui.busy.delete('credential');
      $('#save-key').disabled = false;
      updateCredential(ui.credential);
    }
  });

  $('#api-key').addEventListener('input', () => keyError(''));
  $('#clear-key').addEventListener('click', async () => {
    if (ui.busy.has('credential')) return;
    $('#api-key').value = '';
    ui.busy.add('credential');
    $('#clear-key').disabled = true;
    $('#save-key').disabled = true;
    try {
      const response = await api('/api/credential', { token: '' });
      updateCredential(response.credential);
      keyError('');
      toast('Key cleared. Active and queued jobs are being cancelled.');
      await refresh();
      $('#api-key').focus({ preventScroll: true });
    } catch (error) {
      keyError(error.message);
    } finally {
      ui.busy.delete('credential');
      $('#save-key').disabled = false;
      updateCredential(ui.credential);
    }
  });

  $('#run-all').addEventListener('click', () => start('all'));
  $('#cancel-all').addEventListener('click', async () => {
    ui.busy.add('cancel-all');
    updateButtons();
    try {
      await api('/api/cancel', { all: true });
      await refresh();
      toast('Cancellation requested for all active jobs.');
    } catch (error) {
      showConnectionError(error.message);
    } finally {
      ui.busy.delete('cancel-all');
      updateButtons();
    }
  });
  $('#reconnect').addEventListener('click', () => window.location.reload());
  $('#toggle-bonus').addEventListener('click', () => {
    const show = $('#bonus-sdk').hidden;
    $('#bonus-sdk').hidden = !show;
    $('#toggle-bonus').setAttribute('aria-expanded', String(show));
    $('#toggle-bonus').innerHTML = `${show ? 'Hide' : 'Show'} JavaScript <span aria-hidden="true">${show ? '−' : '+'}</span>`;
  });
  $('#show-pins').addEventListener('click', async () => {
    const output = $('#pins-output');
    if (!output.hidden) { output.hidden = true; return; }
    try {
      const proof = await api('/api/provenance');
      output.textContent = JSON.stringify(proof, null, 2);
      output.hidden = false;
    } catch (error) {
      showConnectionError(error.message);
    }
  });
  window.addEventListener('pagehide', () => { $('#api-key').value = ''; clearTimeout(ui.pollTimer); });
  window.addEventListener('pageshow', event => { if (event.persisted && ui.ready) poll(); });

  async function initialize() {
    $('#run-all').disabled = true;
    try {
      const data = await api('/api/readiness');
      $('#test-banner').hidden = data.mode === 'live';
      renderCollection(data.cards);
      updateCredential(data.credential);
      ui.ready = true;
      ui.connected = true;
      await poll();
    } catch (error) {
      showConnectionError(error.message);
      $('#sdk-groups').setAttribute('aria-busy', 'false');
      setText($('#preflight-label'), 'Local preflight unavailable');
      $('.loading-note')?.replaceChildren(document.createTextNode('Reconnect to load the prepared SDK collection.'));
    }
  }

  initialize();
})();
